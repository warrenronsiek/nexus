module DomainTest exposing (tests)
-- @feature observability-ui
-- @spec docs/features/observability-ui.md

import Expect
import Json.Decode as Decode
import Nexus.Domain exposing (EventClass(..), ProjectScope(..), bucketEvents, dashboardDecoder, eventMatchesScope)
import Test exposing (Test, describe, test)
import Time


tests : Test
tests =
    describe "dashboard domain"
        [ test "decodes typed dashboard windows" <|
            \_ ->
                Decode.decodeString dashboardDecoder dashboardJson
                    |> Result.map
                        (\dashboard ->
                            ( dashboard.counts.activeSessions
                            , List.length dashboard.events.items
                            , dashboard.events.truncated
                            )
                        )
                    |> Expect.equal (Ok ( 2, 2, True ))
        , test "filters events by explicit project scope" <|
            \_ ->
                Decode.decodeString dashboardDecoder dashboardJson
                    |> Result.map
                        (\dashboard ->
                            dashboard.events.items
                                |> List.filter (eventMatchesScope (OneProject "project-a"))
                                |> List.length
                        )
                    |> Expect.equal (Ok 1)
        , test "buckets conflict severity separately from normal activity" <|
            \_ ->
                Decode.decodeString dashboardDecoder dashboardJson
                    |> Result.map
                        (\dashboard ->
                            bucketEvents (Time.millisToPosix 3600000) dashboard.events.items
                                |> List.concatMap .series
                                |> List.map .class
                        )
                    |> Expect.equal (Ok [ Activity, Critical ])
        , test "emits every five-minute bucket across the last hour" <|
            \_ ->
                Decode.decodeString dashboardDecoder dashboardJson
                    |> Result.map
                        (\dashboard ->
                            bucketEvents (Time.millisToPosix 3600000) dashboard.events.items
                                |> List.map (.start >> Time.posixToMillis)
                        )
                    |> Expect.equal
                        (Ok
                            [ 300000
                            , 600000
                            , 900000
                            , 1200000
                            , 1500000
                            , 1800000
                            , 2100000
                            , 2400000
                            , 2700000
                            , 3000000
                            , 3300000
                            , 3600000
                            ]
                        )
        ]


dashboardJson : String
dashboardJson =
    """
    {
      "ok": true,
      "project_id": null,
      "generated_at": "1970-01-01T01:00:00Z",
      "refresh_interval_ms": 2000,
      "counts": {"active_sessions":2,"active_claims":1,"open_conflicts":1,"events":9},
      "events": {
        "truncated": true,
        "items": [
          {"id":2,"project_id":"project-b","session_id":"two","kind":"conflict_detected","payload":{"severity":"critical"},"created_at":"1970-01-01T00:58:00Z"},
          {"id":1,"project_id":"project-a","session_id":"one","kind":"tool_succeeded","payload":{},"created_at":"1970-01-01T00:56:00Z"}
        ]
      },
      "sessions":{"truncated":false,"items":[]},
      "claims":{"truncated":false,"items":[]},
      "conflicts":{"truncated":false,"items":[]}
    }
    """
