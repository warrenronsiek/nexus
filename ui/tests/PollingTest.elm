module PollingTest exposing (tests)
-- @feature observability-ui
-- @spec docs/features/observability-ui.md

import Expect
import Nexus.Polling exposing (DataState(..), begin, canPoll, fail, isCurrentScope, succeed)
import Test exposing (Test, describe, test)


tests : Test
tests =
    describe "polling state"
        [ test "does not overlap an active request" <|
            \_ ->
                begin Loading
                    |> canPoll
                    |> Expect.equal False
        , test "retains the last snapshot when refresh fails" <|
            \_ ->
                Loading
                    |> succeed "snapshot"
                    |> begin
                    |> fail "offline"
                    |> Expect.equal (Stale "snapshot" "offline")
        , test "rejects a response from a superseded project scope" <|
            \_ ->
                isCurrentScope "project-b" "project-a"
                    |> Expect.equal False
        ]
