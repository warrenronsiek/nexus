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
    describe "observed execution identities"
        [ test "containers retain the exact parent chain" <|
            \_ ->
                Waterfall.build root [ node "agent" Nothing "Agent" "host.session" "a", node "tool" (Just "agent") "Read" "tool" "b" ]
                    |> shape
                    |> Expect.equal [ ( "session", Nothing, "emails" ), ( "agent", Nothing, "Agent" ), ( "tool", Just "agent", "Read" ) ]
        , test "repeated calls remain individually selectable" <|
            \_ ->
                Waterfall.build root [ node "a" Nothing "Bash" "tool" "a", node "b" Nothing "Bash" "tool" "b" ]
                    |> List.map .id
                    |> Expect.equal [ "session", "a", "b" ]
        , test "an individual call retains its outcome" <|
            \_ ->
                node "a" Nothing "Bash" "tool" "a" |> Waterfall.summary
                    |> Expect.equal "Bash · tool · elapsed unknown · succeeded"
        ]
