-- @feature observability-ui
-- @feature evals
-- @feature work-history
-- @spec docs/features/observability-ui.md
-- @spec docs/features/evals.md
-- @spec docs/features/work-history.md

module WireTest exposing (tests)

import Expect
import Test exposing (Test, describe, test)
import Wire


tests : Test
tests =
    describe "browser read boundary"
        [ test "wrong work response variant cannot become a dashboard" <|
            \_ ->
                Wire.decode "work_dashboard" "{\"ok\":true,\"response\":\"page\",\"sessions\":[]}"
                    |> Expect.err
        , test "ownership timeline uses its distinct typed response envelope" <|
            \_ ->
                Wire.decode "work_timeline" "{\"ok\":true,\"response\":\"timeline\",\"items\":[],\"next_cursor\":null}"
                    |> Result.map (.rows >> List.length)
                    |> Expect.equal (Ok 0)
        , test "a supplied malformed interval is rejected instead of silently omitted" <|
            \_ ->
                Wire.decode "work_dashboard" "{\"ok\":true,\"response\":\"dashboard\",\"sessions\":[],\"intervals\":[{\"id\":9}]}"
                    |> Expect.err
        , test "semantic activity rows cannot be mistaken for work-activity identities" <|
            \_ ->
                Wire.decode "analytics_summary" "{\"ok\":true,\"activities\":[{\"category_id\":\"search\",\"name\":\"Research\",\"calls\":4,\"actions\":5}]}"
                    |> Result.map (.rows >> List.map .kind)
                    |> Expect.equal (Ok [ "activity_category" ])
        , test "an unmeasured activity kind is labeled unknown rather than zero seconds" <|
            \_ ->
                Wire.decode "work_dashboard" "{\"ok\":true,\"response\":\"dashboard\",\"sessions\":[],\"activity_kinds\":[{\"kind\":\"test\",\"count\":3,\"known_count\":0,\"average_seconds\":null}]}"
                    |> Result.map (.bars >> List.map .detail >> String.join " ")
                    |> Result.map (String.startsWith "Unknown")
                    |> Expect.equal (Ok True)
        , test "projects are titled by their reported repository name and fall back to the ID" <|
            \_ ->
                Wire.decode "projects" "{\"ok\":true,\"truncated\":false,\"projects\":[{\"project_id\":\"7c15a8fa\",\"display_name\":\"comms-platform\",\"worktrees\":[],\"agents\":[],\"last_seen_at\":\"2026-11-05T00:00:00Z\"},{\"project_id\":\"8a89aaef\",\"display_name\":null,\"worktrees\":[],\"agents\":[],\"last_seen_at\":\"2026-11-04T00:00:00Z\"}]}"
                    |> Result.map (.rows >> List.map (\row -> ( row.id, row.title )))
                    |> Expect.equal (Ok [ ( "7c15a8fa", "comms-platform" ), ( "8a89aaef", "8a89aaef" ) ])
        , test "metric literals retain precision while different definitions remain separate" <|
            \_ ->
                Wire.decode "eval_dashboard" "{\"ok\":true,\"action\":\"dashboard\",\"groups\":[],\"series\":[{\"id\":\"v1\",\"label\":\"Latency\",\"unit\":\"seconds\",\"description\":\"Wall time\",\"calculation\":\"end minus start\",\"definition_id\":\"metric-1\",\"points\":[{\"record_id\":\"point\",\"run_id\":\"run\",\"occurred_at\":\"2026-11-01T00:00:00Z\",\"numeric_value\":9007199254740992,\"value\":{\"storage\":\"inline\",\"text\":\"9007199254740993.0000000001\"}}]},{\"id\":\"v2\",\"label\":\"Latency\",\"unit\":\"ms\",\"description\":\"Revised unit\",\"calculation\":\"milliseconds\",\"definition_id\":\"metric-2\",\"points\":[]}]}"
                    |> Result.map (\doc -> ( List.map .definition doc.metrics, List.head doc.metrics |> Maybe.andThen (.points >> List.head) |> Maybe.map .exact ))
                    |> Expect.equal (Ok ( [ "metric-1", "metric-2" ], Just "9007199254740993.0000000001" ))
            , test "coordination sessions are named by display name, then task, then agent and short identity" <|
            \_ ->
                Wire.decode "coordination_session_index" "{\"ok\":true,\"truncated\":false,\"sessions\":[{\"session_id\":\"a\",\"agent\":\"codex\",\"display_name\":\"emails\",\"task_summary\":\"x\",\"conflicts\":2,\"advisories\":3},{\"session_id\":\"b\",\"agent\":\"claude\",\"display_name\":null,\"task_summary\":\"Cache cleanup\",\"conflicts\":0,\"advisories\":0},{\"session_id\":\"0123456789\",\"agent\":\"claude\",\"display_name\":null,\"task_summary\":null,\"conflicts\":0,\"advisories\":0}]}"
                    |> Result.map (.rows >> List.map (\row -> ( row.title, row.subtitle )))
                    |> Expect.equal (Ok [ ( "emails", "2 conflicts · 3 messages" ), ( "Cache cleanup", "0 conflicts · 0 messages" ), ( "claude · 01234567", "0 conflicts · 0 messages" ) ])
        , test "a timeline entry without an identity is rejected" <|
            \_ ->
                Wire.decode "coordination_session_timeline" "{\"ok\":true,\"session\":null,\"entries\":[{\"type\":\"advisory\"}],\"truncated\":false}"
                    |> Expect.err
        , test "memory rows preview the first line of their content" <|
            \_ ->
                Wire.decode "memory_expand" "{\"ok\":true,\"nodes\":[{\"id\":\"m\",\"kind\":\"raw\",\"content\":\"First line\\nSecond line\",\"level\":0,\"span\":1,\"created_at\":\"2026-11-01T00:00:00Z\"}]}"
                    |> Result.map (.rows >> List.map .title)
                    |> Expect.equal (Ok [ "First line" ])
        ]
