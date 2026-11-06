-- @feature observability-ui
-- @feature work-history
-- @spec docs/features/observability-ui.md
-- @spec docs/features/work-history.md

module WaterfallTest exposing (tests)

import Expect
import Json.Encode as E
import Test exposing (Test, describe, test)
import Waterfall


node : String -> Maybe String -> String -> String -> String -> Waterfall.Node
node id parent label kind start =
    { id = id, parent = parent, label = label, kind = kind, start = Just start, end = Just (start ++ "9"), queued = Nothing, count = 1, body = E.object [ ( "outcome", E.string "succeeded" ) ] }


root : Waterfall.Node
root =
    node "session" Nothing "emails" "work.session" "2026-11-01T00:00:00Z"


shape : List Waterfall.Node -> List ( String, Maybe String, String )
shape =
    List.map (\item -> ( item.id, item.parent, item.label ))


tests : Test
tests =
    describe "human-rooted session tree"
        [ test "agent, request and turn containers of a single prompt fold into the session" <|
            \_ ->
                Waterfall.build root
                    [ node "agent" Nothing "claude agent" "host.session" "2026-11-01T00:00:01Z"
                    , node "request" (Just "agent") "Human request" "request" "2026-11-01T00:00:02Z"
                    , node "turn" (Just "request") "Host turn" "host.turn" "2026-11-01T00:00:02Z"
                    , node "edit" (Just "turn") "Edit" "tool" "2026-11-01T00:00:03Z"
                    ]
                    |> shape
                    |> Expect.equal [ ( "session", Nothing, "emails" ), ( "edit", Nothing, "Edit" ) ]
        , test "several prompts stay distinct and are numbered in order" <|
            \_ ->
                Waterfall.build root
                    [ node "second" Nothing "Human request" "request" "2026-11-01T00:05:00Z"
                    , node "first" Nothing "Human request" "request" "2026-11-01T00:01:00Z"
                    , node "turn" (Just "second") "Host turn" "host.turn" "2026-11-01T00:05:00Z"
                    , node "test" (Just "turn") "Router tests" "test.unit" "2026-11-01T00:05:01Z"
                    ]
                    |> shape
                    |> List.sortBy (\( id, _, _ ) -> id)
                    |> Expect.equal [ ( "first", Nothing, "Prompt 1" ), ( "second", Nothing, "Prompt 2" ), ( "session", Nothing, "emails" ), ( "test", Just "second", "Router tests" ) ]
        , test "consecutive repeated leaf calls under one parent become one counted node" <|
            \_ ->
                let
                    built =
                        Waterfall.build root
                            [ node "a" Nothing "Bash" "tool" "2026-11-01T00:00:01Z"
                            , node "b" Nothing "Bash" "tool" "2026-11-01T00:00:02Z"
                            , node "c" Nothing "Bash" "tool" "2026-11-01T00:00:03Z"
                            , node "d" Nothing "Read" "tool" "2026-11-01T00:00:04Z"
                            , node "e" Nothing "Bash" "tool" "2026-11-01T00:00:05Z"
                            ]
                in
                ( shape built, List.map .count built )
                    |> Expect.equal ( [ ( "session", Nothing, "emails" ), ( "group:a", Nothing, "Bash ×3" ), ( "d", Nothing, "Read" ), ( "e", Nothing, "Bash" ) ], [ 1, 3, 1, 1 ] )
        , test "a grouped node summarizes its member outcomes" <|
            \_ ->
                Waterfall.build root [ node "a" Nothing "Bash" "tool" "2026-11-01T00:00:01Z", node "b" Nothing "Bash" "tool" "2026-11-01T00:00:02Z" ]
                    |> List.drop 1
                    |> List.head
                    |> Maybe.map Waterfall.summary
                    |> Expect.equal (Just "Bash ×2 · tool · elapsed unknown · 2/2 succeeded")
        ]
