-- @feature observability-ui
-- @spec docs/features/observability-ui.md

module StateTest exposing (tests)

import Expect
import State exposing (accept, begin, empty)
import Test exposing (Test, describe, test)


tests : Test
tests =
    describe "scoped read lifecycle"
        [ test "late read cannot overwrite a newer scope, previous success survives refresh" <|
            \_ ->
                let
                    first =
                        accept 1 (Ok "old project") (begin empty)

                    newer =
                        begin first

                    late =
                        accept 1 (Ok "late old project") newer
                in
                Expect.equal ( Just "old project", True, 2 ) ( late.value, late.loading, late.generation )
        , test "refresh failure keeps successful data and reports its stale reason" <|
            \_ ->
                empty
                    |> begin
                    |> accept 1 (Ok "evidence")
                    |> begin
                    |> accept 2 (Err "service unavailable")
                    |> (\read -> Expect.equal ( Just "evidence", Just "service unavailable", False ) ( read.value, read.error, read.loading ))
        ]
