// @feature agent-traces
// @spec docs/features/agent-traces.md
// @boundary captured HTTP request and adapter JSON
package nexus.example.koog

import ai.koog.agents.core.agent.execution.AgentExecutionInfo
import ai.koog.agents.core.feature.model.events.LLMCallStartingEvent
import ai.koog.agents.core.feature.model.events.ToolCallStartingEvent
import ai.koog.agents.utils.ModelInfo
import ai.koog.prompt.Prompt
import ai.koog.serialization.JSONObject
import ai.koog.serialization.JSONPrimitive
import com.sun.net.httpserver.HttpServer
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Test
import java.net.InetSocketAddress
import java.net.URI
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

class NexusTracingTest {
    @Test
    fun `a Koog tool event retries the same batch without capturing content by default`() = runBlocking {
        val bodies = LinkedBlockingQueue<String>()
        val attempts = AtomicInteger()
        val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
        server.createContext("/api/v1/agent-traces/events") { exchange ->
            exchange.use {
                assertEquals("POST", exchange.requestMethod)
                assertEquals("Bearer test-secret", exchange.requestHeaders.getFirst("Authorization"))
                assertEquals("test-project", exchange.requestHeaders.getFirst("X-Nexus-Project-ID"))
                assertEquals("support", exchange.requestHeaders.getFirst("X-Nexus-Application-ID"))
                assertEquals("test", exchange.requestHeaders.getFirst("X-Nexus-Environment"))
                bodies.add(exchange.requestBody.bufferedReader().readText())
                exchange.sendResponseHeaders(if (attempts.incrementAndGet() == 1) 503 else 202, -1)
            }
        }
        server.start()
        val processor = NexusTraceProcessor(
            baseUri = URI("http://127.0.0.1:${server.address.port}"),
            bearer = "test-secret",
            projectId = "test-project",
            applicationId = "support",
            environment = "test",
            identify = { runId -> NexusRun(traceId = runId, agentId = "support-agent") }
        )
        try {
            processor.initialize()
            processor.onMessage(toolStart())
            val first = requireNotNull(bodies.poll(3, TimeUnit.SECONDS))
            assertEquals(first, bodies.poll(3, TimeUnit.SECONDS))
            assertFalse(first.contains("omit-me"))
            val batch = Json.parseToJsonElement(first).jsonObject
            assertEquals(1, batch.getValue("schema_version").jsonPrimitive.content.toInt())
            assertEquals("support", batch.getValue("application_id").jsonPrimitive.content)
            val event = batch.getValue("events").jsonArray.single().jsonObject
            assertEquals("support-agent", event.getValue("agent_id").jsonPrimitive.content)
            assertEquals("koog-execution-id", event.getValue("invocation_id").jsonPrimitive.content)
            assertEquals("tool_started", event.getValue("kind").jsonPrimitive.content)
            assertFalse(event.containsKey("payload"))
            assertFalse(event.containsKey("sequence"))
        } finally {
            processor.close()
            server.stop(0)
        }
        assertEquals(1L, processor.stats().delivered)
        assertEquals(0L, processor.stats().dropped)
    }

    @Test
    fun `the app can approve prompts redact tool arguments and record versioned skill evidence`() = runBlocking {
        val requestBodies = LinkedBlockingQueue<String>()
        val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
        server.createContext("/api/v1/agent-traces/events") { exchange ->
            exchange.use {
                requestBodies.add(exchange.requestBody.bufferedReader().readText())
                exchange.sendResponseHeaders(202, -1)
            }
        }
        server.start()
        val run = NexusRun("application-run", "support-agent", "conversation-id")
        val processor = NexusTraceProcessor(
            baseUri = URI("http://127.0.0.1:${server.address.port}"),
            bearer = "test-secret",
            projectId = "test-project",
            applicationId = "support",
            environment = "test",
            identify = { run },
            payloadPolicy = { kind, candidate ->
                when (kind) {
                    "model_started" -> candidate
                    "tool_started" -> {
                        val args = candidate.jsonObject.getValue("toolArgs").jsonObject
                        assertEquals("omit-me", args.getValue("secret").jsonPrimitive.content)
                        JsonObject(mapOf("redacted" to JsonPrimitive(true)))
                    }
                    else -> null
                }
            },
        )
        try {
            processor.initialize()
            processor.onMessage(modelStart())
            processor.onMessage(toolStart())
            processor.recordSkill(run, SkillStage.STARTED, "help", "rules-v7", "skill-call")
            processor.recordSkill(
                run, SkillStage.FINISHED, "help", "rules-v7", "skill-call", TraceOutcome.SUCCEEDED,
            )
        } finally {
            processor.close()
            server.stop(0)
        }
        val encoded = requestBodies.toList()
        assertFalse(encoded.any { it.contains("omit-me") })
        val events = encoded.flatMap { batch ->
            Json.parseToJsonElement(batch).jsonObject.getValue("events").jsonArray
        }.map { it.jsonObject }.associateBy { it.getValue("kind").jsonPrimitive.content }
        val prompt = events.getValue("model_started").getValue("payload").jsonObject
            .getValue("prompt").jsonObject
        assertEquals(JsonPrimitive("order-lookup"), prompt["id"])
        assertEquals(2, prompt.getValue("messages").jsonArray.size)
        val tool = events.getValue("tool_started")
        assertEquals(JsonPrimitive(true), tool.getValue("payload").jsonObject["redacted"])
        val started = events.getValue("skill_started")
        val finished = events.getValue("skill_finished")
        assertEquals(started["invocation_id"], finished["invocation_id"])
        assertEquals(JsonPrimitive("rules-v7"), finished["version"])
        assertEquals(JsonPrimitive("succeeded"), finished["outcome"])
        assertEquals(JsonPrimitive("conversation-id"), finished["conversation_id"])
        assertEquals(4L, processor.stats().delivered)
    }

    private fun modelStart() = LLMCallStartingEvent(
        eventId = "koog-model-id",
        executionInfo = AgentExecutionInfo(null, "model"),
        runId = "application-run",
        prompt = Prompt.build("order-lookup") {
            system("You handle order lookups.")
            user("Where is my order?")
        },
        model = ModelInfo("test", "test-model"),
        tools = listOf("read_order"),
    )

    private fun toolStart() = ToolCallStartingEvent(
        eventId = "koog-execution-id",
        executionInfo = AgentExecutionInfo(null, "tool"),
        runId = "application-run",
        toolCallId = "provider-tool-id",
        toolName = "read_order",
        toolArgs = JSONObject(mapOf("secret" to JSONPrimitive("omit-me"))),
    )
}
