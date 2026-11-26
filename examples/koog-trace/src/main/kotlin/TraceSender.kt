// @feature agent-traces
// @spec docs/features/agent-traces.md
// @boundary bounded trace HTTP batch transport
package nexus.example.koog

import kotlinx.serialization.json.JsonPrimitive
import java.io.IOException
import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import java.time.Duration
import java.util.UUID
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong

internal const val MAX_ENCODED_EVENT_BYTES = 56 * 1024
private const val EVENTS_PER_BATCH = 16
private const val QUEUE_CAPACITY = 128
private const val MAX_SEND_ATTEMPTS = 3

internal class TraceSender(
    baseUri: URI,
    private val bearer: String,
    private val projectId: String,
    private val applicationId: String,
    private val environment: String,
) {
    private val endpoint = baseUri.resolve("/api/v1/agent-traces/events")
    private val client = HttpClient.newBuilder().connectTimeout(Duration.ofSeconds(2)).build()
    private val queue = ArrayBlockingQueue<() -> String>(QUEUE_CAPACITY)
    private val admission = Any()
    @Volatile private var accepting = true
    val delivered = AtomicLong()
    val dropped = AtomicLong()
    private val worker = Thread(::drain, "nexus-koog-traces").apply {
        isDaemon = true
        start()
    }

    fun reject() {
        dropped.incrementAndGet()
    }

    fun offer(event: () -> String): Boolean = synchronized(admission) {
        val queued = accepting && queue.offer(event)
        if (!queued) reject()
        queued
    }

    fun close() {
        synchronized(admission) { accepting = false }
        worker.join(2_000)
        if (worker.isAlive) {
            worker.interrupt()
            worker.join(200)
        }
    }

    private fun drain() {
        try {
            while (accepting || queue.isNotEmpty()) {
                val first = queue.poll(100, TimeUnit.MILLISECONDS) ?: continue
                val events = mutableListOf(first)
                queue.drainTo(events, EVENTS_PER_BATCH - 1)
                send(events)
            }
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        } finally {
            synchronized(admission) {
                accepting = false
                dropped.addAndGet(queue.size.toLong())
                queue.clear()
            }
        }
    }

    private fun send(events: List<() -> String>) {
        var count = 0L
        try {
            val encoded = events.mapNotNull { event ->
                event().takeIf { it.toByteArray(Charsets.UTF_8).size <= MAX_ENCODED_EVENT_BYTES }
            }
            if (encoded.isNotEmpty() && post(batchBody(encoded))) count = encoded.size.toLong()
        } catch (_: IllegalArgumentException) {
            // Invalid local transport configuration loses this batch only.
        } finally {
            delivered.addAndGet(count)
            dropped.addAndGet(events.size - count)
        }
    }

    private fun batchBody(events: List<String>): String = buildString {
        append("{\"schema_version\":1,\"batch_id\":").append(JsonPrimitive(UUID.randomUUID().toString()))
        append(",\"application_id\":").append(JsonPrimitive(applicationId))
        append(",\"environment\":").append(JsonPrimitive(environment))
        append(",\"events\":").append(events.joinToString(",", "[", "]")).append("}")
    }

    private fun post(body: String): Boolean {
        val request = HttpRequest.newBuilder(endpoint)
            .timeout(Duration.ofSeconds(2))
            .header("Authorization", "Bearer $bearer")
            .header("Content-Type", "application/json")
            .header("X-Nexus-Project-ID", projectId)
            .header("X-Nexus-Application-ID", applicationId)
            .header("X-Nexus-Environment", environment)
            .POST(HttpRequest.BodyPublishers.ofString(body))
            .build()
        repeat(MAX_SEND_ATTEMPTS) { attempt ->
            // Reuse precisely the same body and IDs after an uncertain receipt.
            val status = try {
                client.send(request, HttpResponse.BodyHandlers.discarding()).statusCode()
            } catch (_: IOException) {
                null
            }
            if (status != null && status in 200..299) return true
            if (status != null && status != 429 && status !in 500..599) return false
            if (attempt + 1 < MAX_SEND_ATTEMPTS) Thread.sleep(100L shl attempt)
        }
        return false
    }
}
