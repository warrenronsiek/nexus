-- @feature agent-traces
-- @feature observability-ui
-- @spec docs/features/agent-traces.md
-- @spec docs/features/observability-ui.md
-- @boundary trace-flow: pair only explicitly identified lifecycle events within the loaded trace pages.
module TraceFlow exposing (view)

import Html exposing (Html, code, details, div, h3, p, small, span, summary, text)
import Html.Attributes exposing (attribute, class)
import Json.Decode as D
import Json.Encode as E
import Wire


type alias Invocation =
    { key : List String, events : List D.Value }


lifecycle : D.Value -> Maybe ( String, String )
lifecycle event =
    let kind = Wire.str "kind" event in
    [ "agent", "model", "tool", "skill" ]
        |> List.filterMap (\family ->
            if kind == family ++ "_started" then Just ( family, "start" )
            else if kind == family ++ "_finished" then Just ( family, "finish" )
            else Nothing)
        |> List.head


key : D.Value -> List String
key event =
    case lifecycle event of
        Just ( family, _ ) ->
            if Wire.str "invocation_id" event /= "" then
                [ Wire.str "agent_id" event, family, Wire.str "invocation_id" event ]
            else [ "event", Wire.str "event_id" event ]
        Nothing -> [ "event", Wire.str "event_id" event ]


group : D.Value -> List Invocation -> List Invocation
group event invocations =
    let identity = key event in
    if List.any (\invocation -> invocation.key == identity) invocations then
        List.map (\invocation -> if invocation.key == identity then { invocation | events = invocation.events ++ [ event ] } else invocation) invocations
    else invocations ++ [ Invocation identity [ event ] ]


view : String -> (D.Value -> Html msg) -> List D.Value -> Html msg
view selectedEvent evidence events =
    div [ class "trace-execution" ]
        [ h3 [] [ text "Execution flow" ]
        , p [ class "muted" ] [ text "Recorded lifecycle events, in source order. Only matching agent, capability, and invocation IDs are paired. Missing events remain unknown; model-only traces do not establish tool or skill execution." ]
        , div [ class "trace-flow" ] (events |> List.foldl group [] |> List.map (row selectedEvent evidence))
        ]


row : String -> (D.Value -> Html msg) -> Invocation -> Html msg
row selectedEvent evidence invocation =
    let
        events = invocation.events
        first = List.head events |> Maybe.withDefault E.null
        phase desired = List.filter (\event -> lifecycle event |> Maybe.map (Tuple.second >> (==) desired) |> Maybe.withDefault False) events
        starts = phase "start"
        finishes = phase "finish"
        finish = List.head finishes
        family = lifecycle first |> Maybe.map Tuple.first |> Maybe.withDefault (Wire.str "kind" first)
        state =
            if List.length starts > 1 || List.length finishes > 1 then "Ambiguous lifecycle; inspect source events"
            else if List.isEmpty starts && finish /= Nothing then "Start not observed on loaded pages · " ++ (finish |> Maybe.map outcome |> Maybe.withDefault "Unknown outcome")
            else case finish of
                Just completed -> outcome completed
                Nothing -> if lifecycle first /= Nothing then "Finish not observed on loaded pages" else "Recorded event"
        name = events |> List.map (Wire.str "name") |> List.filter ((/=) "") |> List.head |> Maybe.withDefault family
        version = events |> List.map (Wire.str "version") |> List.filter ((/=) "") |> List.head |> Maybe.withDefault "Version not recorded"
        duration = finish |> Maybe.andThen (D.decodeValue (D.field "duration_ms" D.int) >> Result.toMaybe) |> Maybe.map (String.fromInt >> (\ms -> ms ++ " ms")) |> Maybe.withDefault ""
        parent = events |> List.map (Wire.str "parent_event_id") |> List.filter ((/=) "") |> List.head |> Maybe.withDefault ""
    in
    details ([ class "trace-invocation" ] ++ (if selectedEvent /= "" && List.any (Wire.str "event_id" >> (==) selectedEvent) events then [ attribute "open" "" ] else []))
        [ summary []
            [ span [ class "trace-operation" ] [ small [] [ text family ], Html.strong [] [ text name ] ]
            , span [ class "trace-outcome" ] [ text state, small [] [ text duration ] ]
            , small [ class "trace-version" ] [ code [] [ text version ] ]
            ]
        , p [ class "trace-event-meta" ] [ text (Wire.str "agent_id" first ++ " · " ++ Wire.str "occurred_at" first) ]
        , if parent == "" then text "" else p [] [ code [] [ text ("Parent " ++ parent) ] ]
        , div [ class "trace-source-events" ] (List.map evidence events)
        ]


outcome : D.Value -> String
outcome event =
    let raw = Wire.str "outcome" event in
    if raw == "" || raw == "unknown" then "Unknown outcome"
    else String.toUpper (String.left 1 raw) ++ (String.dropLeft 1 raw |> String.replace "_" " ")
