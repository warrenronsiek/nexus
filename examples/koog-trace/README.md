# Send Koog application-agent traces to Nexus

This JVM adapter uses Koog 1.3.0 and Kotlin 2.3.10. Copy the two source files in
`src/main/kotlin` into the application, or build this example with Maven and
a JDK 17 or newer. See the [Nexus agent trace contract](../../docs/features/agent-traces.md)
for server configuration and producer credentials.

Create one processor per agent and install it as a Koog Tracing message processor.
The Nexus API origin and producer credential belong in application configuration.
Use HTTPS when the credential leaves the machine.

```kotlin
import ai.koog.agents.features.tracing.feature.Tracing
import nexus.example.koog.NexusRun
import nexus.example.koog.NexusTraceProcessor
import java.net.URI

val processor = NexusTraceProcessor(
    baseUri = URI.create("https://nexus.example.com"),
    bearer = System.getenv("NEXUS_TRACE_TOKEN"),
    projectId = "support-platform",
    applicationId = "support",
    environment = "production",
    identify = { runId -> NexusRun(traceId = runId, agentId = "order-support") },
)

// Add this inside the agent's feature configuration block.
install(Tracing) {
    addMessageProcessor(processor)
}
```

The example uses Koog's run ID as a trace ID. For an interaction spanning agents
or several Koog runs, have `identify` return the application's trace and
conversation IDs, plus the ID of the configured agent definition that ran this step.
Events from multiple processors use their occurrence timestamps for ordering.
Keep the resolver quick and free of I/O. Use the same reference for any SOS report.

By default, the adapter uploads event metadata. Pass
`payloadPolicy = { kind, candidate -> /* return redacted JSON or null */ }` to
select which content is approved for storage. The callback runs on the uploader.
Candidates include Koog prompt snapshots, structured model output, complete
streamed text/tool frames, tool arguments and results, and errors. If an app
rewrites the prompt in a start interceptor, successful completion carries the
effective prompt. To capture the exact prompt on failure or cover model calls
made outside Koog's traced executor, add a recorder at the application's model
boundary. `recordMessage` supplies application messages that are only available
outside the model buffer.

Koog's skill catalog makes definitions available through the system prompt.
Instrument the app's loader or activation tool for exact evidence. For example,
log a loaded version and use a fresh invocation ID for each activation.

```kotlin
import nexus.example.koog.SkillStage
import nexus.example.koog.TraceOutcome
import java.util.UUID

val run = NexusRun("order-981-run", "order-support")
processor.recordSkill(run, SkillStage.LOADED, "order-history", "2.1")
val invocation = UUID.randomUUID().toString()
processor.recordSkill(run, SkillStage.STARTED, "order-history", "2.1", invocation)
// Execute the application's skill and record its real outcome.
processor.recordSkill(
    run, SkillStage.FINISHED, "order-history", "2.1", invocation, TraceOutcome.SUCCEEDED,
)
```

The in-memory queue holds at most 128 events. The worker ships up to 16 events per
request and resends the same body and IDs after a transient delivery error.
It drops oversized content while preserving its event metadata. After queue
saturation, exhausted retries, terminal rejection, or a bounded shutdown,
`stats()` exposes the dropped-event count and payload omissions. Batches still
in memory can be lost on process termination. Close the processor when the
agent is closed, and use a durable application outbox when those losses matter.

Run the focused adapter test from the Nexus repository root. It exercises
Koog's tool event type and verifies the actual HTTP request and retry body.

```sh
mvn -f examples/koog-trace/pom.xml test
```
