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
        , test "metric literals retain precision while different definitions remain separate" <|
            \_ ->
                Wire.decode "eval_dashboard" "{\"ok\":true,\"action\":\"dashboard\",\"groups\":[],\"series\":[{\"id\":\"v1\",\"label\":\"Latency\",\"unit\":\"seconds\",\"description\":\"Wall time\",\"calculation\":\"end minus start\",\"definition_id\":\"metric-1\",\"points\":[{\"record_id\":\"point\",\"run_id\":\"run\",\"occurred_at\":\"2026-11-01T00:00:00Z\",\"numeric_value\":9007199254740992,\"value\":{\"storage\":\"inline\",\"text\":\"9007199254740993.0000000001\"}}]},{\"id\":\"v2\",\"label\":\"Latency\",\"unit\":\"ms\",\"description\":\"Revised unit\",\"calculation\":\"milliseconds\",\"definition_id\":\"metric-2\",\"points\":[]}]}"
                    |> Result.map (\doc -> ( List.map .definition doc.metrics, List.head doc.metrics |> Maybe.andThen (.points >> List.head) |> Maybe.map .exact ))
                    |> Expect.equal (Ok ( [ "metric-1", "metric-2" ], Just "9007199254740993.0000000001" ))
        ]
