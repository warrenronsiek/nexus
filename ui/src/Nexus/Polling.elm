-- @feature observability-ui
-- @spec docs/features/observability-ui.md
module Nexus.Polling exposing (DataState(..), begin, canPoll, fail, isCurrentScope, succeed, value)


type DataState data
    = Loading
    | Refreshing data
    | Fresh data
    | Stale data String
    | Failed String


begin : DataState data -> DataState data
begin state =
    case state of
        Fresh data ->
            Refreshing data

        Stale data _ ->
            Refreshing data

        Failed _ ->
            Loading

        _ ->
            state


succeed : data -> DataState data -> DataState data
succeed data _ =
    Fresh data


fail : String -> DataState data -> DataState data
fail message state =
    case state of
        Refreshing data ->
            Stale data message

        Fresh data ->
            Stale data message

        Stale data _ ->
            Stale data message

        _ ->
            Failed message


canPoll : DataState data -> Bool
canPoll state =
    case state of
        Loading ->
            False

        Refreshing _ ->
            False

        _ ->
            True


isCurrentScope : scope -> scope -> Bool
isCurrentScope current response =
    current == response


value : DataState data -> Maybe data
value state =
    case state of
        Refreshing data ->
            Just data

        Fresh data ->
            Just data

        Stale data _ ->
            Just data

        _ ->
            Nothing
