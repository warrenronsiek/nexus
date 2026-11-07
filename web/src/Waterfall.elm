-- @feature observability-ui
-- @feature work-history
-- @spec docs/features/observability-ui.md
-- @spec docs/features/work-history.md
module Waterfall exposing (Node, summary)

{-| Retains individual observed activities and their exact parentage for execution flow. -}

import Json.Decode as D


type alias Node =
    { id : String, parent : Maybe String, label : String, kind : String, start : Maybe String, end : Maybe String, queued : Maybe String, body : D.Value }


{-| A one-line description of a selected node: kind, recorded elapsed time and outcome. -}
summary : Node -> String
summary node =
    let
        elapsed = elapsedSeconds node.body |> Maybe.map duration |> Maybe.withDefault "elapsed unknown"
        outcome = D.decodeValue (D.field "outcome" D.string) node.body |> Result.withDefault "unknown"
    in
    String.join " · " [ node.label, node.kind, elapsed, outcome ]


elapsedSeconds : D.Value -> Maybe Float
elapsedSeconds body =
    D.decodeValue (D.at [ "elapsed", "nanoseconds" ] D.string) body |> Result.toMaybe |> Maybe.andThen String.toFloat |> Maybe.map (\ns -> ns / 1.0e9)


duration : Float -> String
duration seconds =
    if seconds >= 3600 then String.fromFloat (toFloat (round (seconds / 36)) / 100) ++ "h" else if seconds >= 60 then String.fromFloat (toFloat (round (seconds / 6)) / 10) ++ "m" else String.fromFloat (toFloat (round (seconds * 100)) / 100) ++ "s"
