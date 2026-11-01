-- @feature observability-ui
-- @spec docs/features/observability-ui.md
-- @boundary browser-json: validate the closed response envelopes and presentation records; retain original response text for exact immutable evidence.
module Wire exposing (Bar, Document, Interval, Metric, Point, Row, decode, field, list, number, optional, refreshInterval, str, strings, valueAt)

import Json.Decode as D exposing (Decoder, Value)
import Json.Encode as E
import Dict


type alias Row =
    { id : String, title : String, subtitle : String, kind : String, body : Value }


type alias Bar =
    { id : String, label : String, value : Maybe Float, detail : String }


type alias Point =
    { id : String, label : String, at : String, value : Maybe Float, exact : String }


type alias Interval =
    { id : String, parent : Maybe String, label : String, kind : String, start : Maybe String, end : Maybe String, queued : Maybe String, body : Value }


type alias Metric =
    { id : String, label : String, unit : String, description : String, calculation : String, definition : String, points : List Point }


type alias Document =
    { raw : String, root : Value, rows : List Row, bars : List Bar, points : List Point, intervals : List Interval, metrics : List Metric, notices : List String, next : Maybe Value }


field : String -> Value -> Value
field key root =
    valueAt [ key ] root


valueAt : List String -> Value -> Value
valueAt path root =
    D.decodeValue (D.at path D.value) root |> Result.withDefault E.null


str : String -> Value -> String
str key root =
    D.decodeValue (D.field key D.string) root |> Result.withDefault ""


number : String -> Value -> Float
number key root =
    D.decodeValue (D.field key D.float) root |> Result.withDefault 0


list : String -> Value -> List Value
list key root =
    D.decodeValue (D.field key (D.list D.value)) root |> Result.withDefault []


strings : String -> Value -> List String
strings key root =
    D.decodeValue (D.field key (D.list D.string)) root |> Result.withDefault []


refreshInterval : Value -> Result String Float
refreshInterval root =
    D.decodeValue
        (D.field "refresh_interval_ms" D.int
            |> D.andThen
                (\milliseconds ->
                    if milliseconds > 0 && milliseconds <= 2147483647 then
                        D.succeed (toFloat milliseconds)
                    else
                        D.fail "The configured refresh interval is outside the browser timer range."
                )
        ) root
        |> Result.mapError D.errorToString


nullable : String -> Decoder (Maybe String)
nullable key =
    optional key (D.nullable D.string) Nothing


optional : String -> Decoder a -> a -> Decoder a
optional key decoder fallback =
    D.keyValuePairs D.value
        |> D.andThen (\entries -> if Dict.member key (Dict.fromList entries) then D.field key decoder else D.succeed fallback)


row : String -> String -> String -> Decoder Row
row idField titleField kind =
    D.map3 (\id title body -> Row id title "" kind body) (D.field idField D.string) (D.field titleField D.string) D.value


withSubtitle : (Value -> String) -> Decoder Row -> Decoder Row
withSubtitle describe decoder =
    D.map (\item -> { item | subtitle = describe item.body }) decoder


workRow : Decoder Row
workRow =
    D.map4 (\id scope title body -> Row id title (str "kind" body ++ " · " ++ str "state" body) scope body)
        (D.at [ "subject", "id" ] D.string)
        (D.at [ "subject", "scope" ] D.string)
        (D.field "title" D.string)
        D.value


coordinationSession : Decoder Row
coordinationSession =
    D.map4 (\id title agent body -> Row id (Maybe.withDefault (agent ++ " · " ++ id) title) (str "task_summary" body) "session" body)
        (D.field "session_id" D.string) (nullable "display_name") (D.field "agent" D.string) D.value


interval : Decoder Interval
interval =
    D.map8 Interval (D.field "id" D.string) (nullable "parent_activity_id") (D.field "title" D.string) (D.field "kind" D.string)
        (nullable "started_at") (nullable "finished_at") (nullable "queued_at") D.value


point : Decoder Point
point =
    D.map5 Point (D.field "record_id" D.string) (D.field "run_id" D.string) (D.field "occurred_at" D.string)
        (D.field "numeric_value" (D.nullable D.float))
        (D.field "value" (D.oneOf [ D.field "text" D.string, D.map (\id -> "Artifact: " ++ id) (D.field "artifact_id" D.string), D.field "reason" D.string ]))


metric : Decoder Metric
metric =
    D.map7 Metric (D.field "id" D.string) (D.field "label" D.string) (D.field "unit" D.string)
        (D.field "description" D.string) (D.field "calculation" D.string) (D.field "definition_id" D.string) (D.field "points" (D.list point))


required : String -> Decoder a -> Value -> Result D.Error a
required key decoder root =
    D.decodeValue (D.field key decoder) root


decode : String -> String -> Result String Document
decode kind raw =
    D.decodeString
        (D.field "ok" D.bool
            |> D.andThen
                (\ok ->
                    if ok then
                        D.value

                    else
                        D.fail "The service rejected this read."
                )
        )
        raw
        |> Result.andThen (\root -> validateEnvelope kind root |> Result.andThen (\_ -> shape kind root) |> Result.map (\doc -> { doc | raw = raw }))
        |> Result.mapError D.errorToString


validateEnvelope : String -> Value -> Result D.Error ()
validateEnvelope kind root =
    let
        expected =
            case kind of
                "work_dashboard" -> Just ( "response", "dashboard" )
                "work_list" -> Just ( "response", "page" )
                "work_activities" -> Just ( "response", "page" )
                "work_timeline" -> Just ( "response", "timeline" )
                "work_get" -> Just ( "response", "detail" )
                "work_events" -> Just ( "response", "events" )
                "concern_dashboard" -> Just ( "response", "dashboard" )
                "concern_list" -> Just ( "response", "page" )
                "concern_get" -> Just ( "response", "detail" )
                "concern_history" -> Just ( "response", "history" )
                "eval_dashboard" -> Just ( "action", "dashboard" )
                "eval_list" -> Just ( "action", "listed" )
                "eval_get" -> Just ( "action", "detailed" )
                "eval_why" -> Just ( "action", "history" )
                _ -> Nothing
    in
    case expected of
        Nothing -> Ok ()
        Just ( key, tag ) ->
            D.decodeValue (D.field key D.string |> D.andThen (\actual -> if actual == tag then D.succeed () else D.fail ("Expected " ++ tag ++ " response"))) root


shape : String -> Value -> Result D.Error Document
shape kind root =
    let
        base =
            Document "" root [] [] [] [] [] (strings "notices" root) (D.decodeValue (D.field "next_cursor" (D.nullable D.value)) root |> Result.withDefault Nothing)

        rows key decoder =
            required key (D.list decoder) root |> Result.map (\items -> { base | rows = items })

        described key title =
            rows key (withSubtitle (\body -> title body) (row "id" "title" key))

        work =
            required "sessions" (D.list workRow) root
                |> Result.andThen
                    (\sessions ->
                        D.decodeValue (optional "intervals" (D.list interval) []) root
                            |> Result.map
                                (\intervals ->
                                    { base
                                        | rows = sessions
                                        , intervals = intervals
                                        , bars = list "activity_kinds" root |> List.map (\v -> Bar (str "kind" v) (str "kind" v) (if number "known_count" v > 0 then Just (number "average_seconds" v) else Nothing) ((if number "known_count" v > 0 then duration (number "average_seconds" v) else "Unknown") ++ " avg · " ++ String.fromFloat (number "known_count" v) ++ "/" ++ String.fromFloat (number "count" v) ++ " timed"))
                                        , points = list "daily" root |> List.map (\v -> Point (str "date" v) (str "date" v) (str "date" v) (if number "known_count" v > 0 then Just (number "average_seconds" v) else Nothing) "")
                                    }
                                )
                    )
    in
    case kind of
        "projects" ->
            rows "projects" (withSubtitle (\v -> String.join " · " (strings "worktrees" v)) (row "project_id" "project_id" "project"))

        "coordination" ->
            D.decodeValue (D.at [ "sessions", "items" ] (D.list coordinationSession)) root
                |> Result.map (\items -> { base | rows = items })

        "coordination_sessions" ->
            rows "sessions" coordinationSession

        "coordination_claims" ->
            rows "claims" (row "id" "path" "claim")

        "coordination_conflicts" ->
            rows "conflicts" (row "id" "path" "conflict")

        "coordination_events" ->
            rows "events" (D.map2 (\body title -> Row (String.fromFloat (number "id" body)) title (str "created_at" body) "event" body) D.value (D.field "kind" D.string))

        "work_dashboard" ->
            work

        "work_list" ->
            rows "items" workRow

        "work_activities" ->
            rows "items" workRow

        "work_timeline" ->
            rows "items" workRow

        "work_events" ->
            rows "observations" (row "id" "occurred_at" "observation")

        "eval_dashboard" ->
            required "groups" (D.list (withSubtitle (\v -> String.join ", " (strings "models" v) ++ " · " ++ String.fromFloat (number "run_count" v) ++ " runs") (row "id" "label" "eval_group"))) root
                |> Result.andThen (\items -> required "series" (D.list metric) root |> Result.map (\metrics -> { base | rows = items, metrics = metrics }))

        "eval_list" ->
            rows "records" (withSubtitle (\v -> str "kind" v ++ " · " ++ str "occurred_at" v) (row "id" "title" "eval_record"))

        "eval_why" ->
            rows "records" (withSubtitle (str "occurred_at") (row "id" "title" "eval_record"))

        "papercuts" ->
            described "papercuts" (\v -> String.fromFloat (number "upvote_count" v) ++ " encounters · " ++ str "updated_at" v)

        "sos_list" ->
            described "issues" (\v -> String.fromFloat (number "importance_count" v) ++ " reports · " ++ str "last_reported_at" v)

        "analytics_summary" ->
            rows "activities" (withSubtitle (\v -> String.fromFloat (number "calls" v) ++ " calls · " ++ String.fromFloat (number "actions" v) ++ " operations") (row "category_id" "name" "activity_category"))
                |> Result.map (\doc -> { doc | bars = List.map (\r -> Bar r.id r.title (Just (number "actions" r.body)) r.subtitle) doc.rows })

        "analytics_catalog" ->
            rows "capabilities" (withSubtitle (\v -> String.fromFloat (number "calls" v) ++ " calls · " ++ String.fromFloat (number "offered_sessions" v) ++ " offered sessions · " ++ str "origin" v) (row "resource_key" "name" "capability"))

        "analytics_opportunities" ->
            rows "opportunities" (withSubtitle (\v -> String.fromFloat (number "calls" v) ++ " ad hoc calls · " ++ str "evidence" v) (row "category_id" "name" "opportunity"))

        "analytics_examples" ->
            rows "examples" (D.map2 (\name body -> Row name name (str "description" body) "example" body) (D.field "name" D.string) D.value)

        "usage" ->
            D.decodeValue
                (D.map2 (++)
                    (D.field "skills" (D.list (withSubtitle (\v -> String.fromFloat (number "count" v) ++ " invocations · " ++ String.fromFloat (number "sessions" v) ++ " sessions") (row "name" "name" "skill"))))
                    (D.field "tools" (D.list (withSubtitle (\v -> String.fromFloat (number "count" v) ++ " invocations · " ++ String.fromFloat (number "sessions" v) ++ " sessions") (row "name" "name" "tool"))))
                ) root
                |> Result.map (\items -> { base | rows = items, bars = List.map (\r -> Bar r.id r.title (Just (number "count" r.body)) r.subtitle) items })

        "concern_dashboard" ->
            rows "areas" (D.map2 (\body area -> Row (str "id" area) (str "display_name" area) (str "kind" area ++ " · " ++ String.fromFloat (number "work_sessions" (field "counts" body)) ++ " sessions affected") "concern_area" body) D.value (D.field "area" D.value))
                |> Result.map (\doc -> { doc | bars = List.map (\r -> Bar r.id r.title (Just (number "work_sessions" (field "counts" r.body))) r.subtitle) doc.rows, points = list "daily" root |> List.map (\v -> Point (str "date" v) (str "date" v) (str "date" v) (Just (number "raw_observations" v)) "") })

        "concern_list" ->
            described "items" (\v -> str "status" v ++ " · " ++ str "last_observed_at" v)

        "concern_history" ->
            rows "observations" (D.map2 (\body input -> Row (str "id" input) (str "event" (field "body" input)) (str "observed_at" input) "concern_observation" body) D.value (D.field "input" D.value))

        "memory_context" ->
            D.decodeValue (D.at [ "context", "nodes" ] (D.list (D.map2 (\body id -> Row id (String.left 100 (str "content" body)) (str "kind" body ++ " · " ++ str "created_at" body) (str "kind" body) body) D.value (D.field "id" D.string)))) root |> Result.map (\items -> { base | rows = items })

        "memory_search" ->
            rows "memories" (D.map2 (\body id -> Row id (String.left 100 (str "content" body)) (str "created_at" body) "raw" body) D.value (D.field "id" D.string))

        "memory_expand" ->
            rows "nodes" (D.map2 (\body id -> Row id (String.left 100 (str "content" body)) (str "created_at" body) (str "kind" body) body) D.value (D.field "id" D.string))

        _ ->
            Ok base


duration : Float -> String
duration seconds =
    if seconds >= 3600 then
        String.fromFloat (toFloat (round (seconds / 36)) / 100) ++ "h"

    else if seconds >= 60 then
        String.fromFloat (toFloat (round (seconds / 6)) / 10) ++ "m"

    else
        String.fromFloat (toFloat (round (seconds * 100)) / 100) ++ "s"
