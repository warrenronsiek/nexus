// @feature agent-traces
// @spec docs/features/agent-traces.md
// @boundary native Koog feature messages and optional caller-approved JSON payloads
package nexus.example.koog

import ai.koog.agents.core.feature.message.FeatureMessage
import ai.koog.agents.core.feature.message.FeatureMessageProcessor
import ai.koog.agents.core.feature.model.events.AgentCompletedEvent
import ai.koog.agents.core.feature.model.events.AgentExecutionFailedEvent
import ai.koog.agents.core.feature.model.events.AgentStartingEvent
import ai.koog.agents.core.feature.model.events.DefinedFeatureEvent
import ai.koog.agents.core.feature.model.events.LLMCallCompletedEvent
import ai.koog.agents.core.feature.model.events.LLMCallFailedEvent
import ai.koog.agents.core.feature.model.events.LLMCallStartingEvent
import ai.koog.agents.core.feature.model.events.LLMStreamingFailedEvent
import ai.koog.agents.core.feature.model.events.LLMStreamingFrameReceivedEvent
import ai.koog.agents.core.feature.model.events.LLMStreamingStartingEvent
import ai.koog.agents.core.feature.model.events.ToolCallCompletedEvent
import ai.koog.agents.core.feature.model.events.ToolCallFailedEvent
import ai.koog.agents.core.feature.model.events.ToolCallStartingEvent
import ai.koog.agents.core.feature.model.events.ToolValidationFailedEvent
import ai.koog.agents.core.feature.remote.server.config.DefaultServerConnectionConfig
import ai.koog.prompt.streaming.StreamFrame
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.serializer
import java.net.URI
import java.time.Instant
import java.util.UUID
import java.util.concurrent.atomic.AtomicLong

data class NexusRun(
    val traceId: String,
    val agentId: String,
    val conversationId: String? = null,
    val turnId: String? = null,
)

enum class SkillStage(internal val eventKind: String) {
    OFFERED("skill_offered"), LOADED("skill_loaded"),
    STARTED("skill_started"), FINISHED("skill_finished"),
}

enum class TraceOutcome(internal val wire: String) {
    SUCCEEDED("succeeded"), FAILED("failed"), CANCELED("canceled"),
    VALIDATION_FAILED("validation_failed"), REJECTED("rejected"), TIMED_OUT("timed_out"), UNKNOWN("unknown"),
}

data class TraceStats(val delivered: Long, val dropped: Long, val omittedPayloads: Long)

/**
 * Install as a message processor for Koog Tracing. The run resolver is called
 * on the capture path. The payload policy runs in the upload worker and may
 * redact the candidate or return null to omit it. Omit the policy for metadata only.
 */
class NexusTraceProcessor(
    baseUri: URI,
    bearer: String,
    projectId: String,
    applicationId: String,
    environment: String,
    private val identify: (String) -> NexusRun,
    private val payloadPolicy: ((eventKind: String, candidate: JsonElement) -> JsonElement?)? = null,
) : FeatureMessageProcessor() {
    private val omittedPayloads = AtomicLong()
    private val sender = TraceSender(baseUri, bearer, projectId, applicationId, environment)

    override suspend fun processMessage(message: FeatureMessage) {
        if (message !is DefinedFeatureEvent) return
        val fields = fields(message) ?: return
        val run = try {
            identify(fields.runId)
        } catch (_: Exception) {
            sender.reject()
            return
        }
        capture(run, fields, Instant.ofEpochMilli(message.timestamp)) { koogPayload(message) }
    }

    /** Handle reload versus actual activation explicitly at the application boundary. */
    fun recordSkill(
        run: NexusRun,
        stage: SkillStage,
        name: String,
        version: String?,
        invocationId: String? = null,
        outcome: TraceOutcome? = null,
        parentEventId: String? = null,
        payload: JsonElement? = null,
    ): String? {
        if (stage in setOf(SkillStage.STARTED, SkillStage.FINISHED) && invocationId == null) {
            sender.reject()
            return null
        }
        val fields = EventFields("", stage.eventKind, invocationId, name, outcome)
        return capture(run, fields, Instant.now(), version, parentEventId) { payload }
    }

    /** Add an application message when it is available outside the Koog model buffer. */
    fun recordMessage(run: NexusRun, role: String, content: String): String? =
        capture(run, EventFields("", "message", name = role), Instant.now()) {
            buildJsonObject { put("role", role); put("content", content) }
        }

    fun stats(): TraceStats = TraceStats(sender.delivered.get(), sender.dropped.get(), omittedPayloads.get())

    override suspend fun close() {
        withContext(Dispatchers.IO) { sender.close() }
        super.close()
    }

    private fun capture(
        run: NexusRun,
        fields: EventFields,
        occurredAt: Instant,
        version: String? = null,
        parentEventId: String? = null,
        candidate: () -> JsonElement?,
    ): String? {
        val eventId = UUID.randomUUID().toString()
        val meta = buildJsonObject {
            put("event_id", eventId)
            put("trace_id", run.traceId)
            put("agent_id", run.agentId)
            put("kind", fields.kind)
            put("occurred_at", occurredAt.toString())
            run.conversationId?.let { put("conversation_id", it) }
            run.turnId?.let { put("turn_id", it) }
            fields.invocationId?.let { put("invocation_id", it) }
            fields.name?.let { put("name", it) }
            fields.outcome?.let { put("outcome", it.wire) }
            version?.let { put("version", it) }
            parentEventId?.let { put("parent_event_id", it) }
        }
        val queued = if (payloadPolicy == null) {
            sender.offer(meta::toString)
        } else {
            sender.offer { encodedEvent(meta, fields.kind, candidate) }
        }
        return eventId.takeIf { queued }
    }

    private fun encodedEvent(meta: JsonObject, kind: String, candidate: () -> JsonElement?): String {
        val policy = payloadPolicy ?: return meta.toString()
        try {
            val payload = candidate()?.let { policy(kind, it) } ?: return meta.toString()
            val fullEvent = JsonObject(meta + ("payload" to payload)).toString()
            if (fullEvent.toByteArray(Charsets.UTF_8).size <= MAX_ENCODED_EVENT_BYTES) return fullEvent
        } catch (_: Exception) {
            // Failed policy or serialization keeps the metadata and drops only the payload.
        }
        omittedPayloads.incrementAndGet()
        return meta.toString()
    }

    private fun fields(event: DefinedFeatureEvent): EventFields? = when (event) {
        is AgentStartingEvent -> EventFields(event.runId, "agent_started")
        is AgentCompletedEvent -> EventFields(event.runId, "agent_finished", outcome = TraceOutcome.SUCCEEDED)
        is AgentExecutionFailedEvent -> EventFields(event.runId, "agent_finished", outcome = TraceOutcome.FAILED)
        is LLMCallStartingEvent -> EventFields(event.runId, "model_started", event.eventId, event.model.model)
        is LLMCallCompletedEvent -> EventFields(
            event.runId, "model_finished", event.eventId, event.model.model, TraceOutcome.SUCCEEDED,
        )
        is LLMCallFailedEvent -> EventFields(
            event.runId, "model_finished", event.eventId, event.model.model, TraceOutcome.FAILED,
        )
        is LLMStreamingStartingEvent -> EventFields(event.runId, "model_started", event.eventId, event.model.model)
        is LLMStreamingFailedEvent -> EventFields(
            event.runId, "model_finished", event.eventId, event.model.model, TraceOutcome.FAILED,
        )
        is LLMStreamingFrameReceivedEvent -> streamFields(event)
        is ToolCallStartingEvent -> EventFields(event.runId, "tool_started", event.eventId, event.toolName)
        is ToolCallCompletedEvent -> EventFields(
            event.runId, "tool_finished", event.eventId, event.toolName, TraceOutcome.SUCCEEDED,
        )
        is ToolCallFailedEvent -> EventFields(
            event.runId, "tool_finished", event.eventId, event.toolName, TraceOutcome.FAILED,
        )
        is ToolValidationFailedEvent -> EventFields(
            event.runId, "tool_finished", event.eventId, event.toolName, TraceOutcome.VALIDATION_FAILED,
        )
        else -> null
    }

    // A native streaming completion callback also runs after failures. The model's
    // End frame gives stronger evidence. Complete frames retain content without token deltas.
    private fun streamFields(event: LLMStreamingFrameReceivedEvent): EventFields? = when (event.frame) {
        is StreamFrame.End -> EventFields(
            event.runId, "model_finished", event.eventId, event.model.model, TraceOutcome.SUCCEEDED,
        )
        is StreamFrame.TextComplete, is StreamFrame.ToolCallComplete ->
            payloadPolicy?.let { EventFields(event.runId, "message", event.eventId, "assistant") }
        else -> null
    }
}

private data class EventFields(
    val runId: String,
    val kind: String,
    val invocationId: String? = null,
    val name: String? = null,
    val outcome: TraceOutcome? = null,
)

private val koogJson = DefaultServerConnectionConfig().jsonConfig

private fun koogPayload(event: DefinedFeatureEvent): JsonObject? {
    val allowedFields = when (event) {
        is AgentCompletedEvent -> setOf("result")
        is AgentExecutionFailedEvent -> setOf("error")
        is LLMCallStartingEvent, is LLMStreamingStartingEvent -> setOf("prompt", "model", "tools")
        is LLMCallCompletedEvent -> setOf("prompt", "model", "response", "moderationResponse")
        is LLMCallFailedEvent, is LLMStreamingFailedEvent -> setOf("prompt", "model", "error")
        is LLMStreamingFrameReceivedEvent -> setOf("frame")
        is ToolCallStartingEvent -> setOf("toolCallId", "toolArgs")
        is ToolCallCompletedEvent -> setOf("toolCallId", "result")
        is ToolCallFailedEvent, is ToolValidationFailedEvent -> setOf("toolCallId", "message", "error")
        else -> return null
    }
    val serializer = koogJson.serializersModule.serializer<FeatureMessage>()
    val native = koogJson.encodeToJsonElement(serializer, event).jsonObject
    return JsonObject(native.filterKeys(allowedFields::contains))
}
