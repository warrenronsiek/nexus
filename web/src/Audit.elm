-- @feature audit-registry
-- @feature observability-ui
-- @spec docs/features/audit-registry.md
-- @spec docs/features/observability-ui.md
-- @boundary audit-read: inspect the closed audit list and detail responses.
module Audit exposing (Field(..), Filters, Scope, charts, fields, listView, parseScope, recordView, routeFields, selectBucket, set, toolbar)

import Dict
import Html exposing (Html, a, button, code, dd, details, div, dl, dt, form, h2, h3, input, label, p, section, small, span, summary, text)
import Html.Attributes exposing (attribute, class, disabled, href, id, name, target, type_, value)
import Html.Events exposing (onClick, onInput, onSubmit)
import Json.Decode as D
import Json.Encode as E
import Wire exposing (Bar, Document, Row)


type alias Filters =
    { control : String, version : String, from : String, until : String }


type alias Scope =
    { filters : Filters, cutoff : Maybe Int }


parseScope : Dict.Dict String String -> Scope
parseScope params =
    let
        get key = Dict.get key params |> Maybe.withDefault ""
    in
    { filters = Filters (get "control") (get "version") (get "from") (get "until")
    , cutoff = Dict.get "cutoff" params |> Maybe.andThen String.toInt
    }


routeFields : Scope -> List ( String, String )
routeFields scope =
    let
        filters = scope.filters
    in
    [ ( "control", filters.control )
    , ( "version", if String.trim filters.control == "" then "" else filters.version )
    , ( "from", filters.from )
    , ( "until", filters.until )
    , ( "cutoff", scope.cutoff |> Maybe.map String.fromInt |> Maybe.withDefault "" )
    ]
        |> List.filter (\( _, value ) -> String.trim value /= "")

type Field
    = Control
    | Version
    | From
    | Until


set : Field -> String -> Filters -> Filters
set field content current =
    case field of
        Control -> { current | control = content }
        Version -> { current | version = content }
        From -> { current | from = content }
        Until -> { current | until = content }


fields : Filters -> List ( String, E.Value )
fields filters =
    let
        optional key val =
            Maybe.map (Tuple.pair key) val

        string content =
            if String.trim content == "" then Nothing else Just (E.string (String.trim content))

        date content =
            string (if content == "" then "" else content ++ "T00:00:00Z")
    in
    List.filterMap identity
        [ optional "control_key" (string filters.control)
        , optional "version" (if String.trim filters.control == "" then Nothing else String.toInt filters.version |> Maybe.map E.int)
        , optional "from" (date filters.from)
        , optional "until" (date filters.until)
        ]


toolbar : (Field -> String -> msg) -> msg -> Bool -> Filters -> Html msg
toolbar inputChanged apply loading filters =
    let
        field title key kind fieldName extra current =
            label [ class "scope-control" ]
                [ span [ class "muted" ] [ text title ]
                , input ([ name key, type_ kind, value current, onInput (inputChanged fieldName) ] ++ extra) []
                ]
    in
    form [ class "audit-filters", onSubmit apply ]
        [ field "Control key" "audit-control" "text" Control [ attribute "maxlength" "160" ] filters.control
        , field "Version" "audit-version" "number" Version [ attribute "min" "1", disabled (String.trim filters.control == "") ] filters.version
        , field "From (UTC)" "audit-from" "date" From [] filters.from
        , field "Before (UTC)" "audit-until" "date" Until [] filters.until
        , button [ type_ "submit", disabled loading ] [ text "Apply filters" ]
        ]


type alias Bucket =
    { chart : String, key : String, label : String, count : Float, order : Float }


chartSpecs : List ( String, String )
chartSpecs =
    [ ( "audit-days", "Runs by UTC day" )
    , ( "audit-controls", "Runs by control" )
    , ( "audit-results", "Categorical results" )
    , ( "audit-followups", "Finding dispositions" )
    ]


buckets : Row -> List Bucket
buckets row =
    let
        body = row.body
        control = Wire.field "control" body
        controlKey = Wire.str "key" control
        day = String.left 10 (Wire.str "occurred_at" body)
        version = String.fromFloat (Wire.number "version" control)
        code_ = Wire.str "result_code" body
        label_ = Wire.str "result_label" body
        codeKey = E.list E.string [ controlKey, version, code_ ] |> E.encode 0
        resultOrder = D.decodeValue (D.field "result_order" D.float) body |> Result.withDefault 0
        dispositions =
            D.decodeValue (D.field "finding_dispositions" (D.dict D.float)) body
                |> Result.withDefault Dict.empty
        unknown = Wire.number "findings" body - (Dict.values dispositions |> List.sum)
        followups =
            (if unknown > 0 then Dict.insert "unrecorded" unknown dispositions else dispositions)
                |> Dict.toList
                |> List.map (\( state, count ) -> Bucket "audit-followups" state (caption state) count 0)
        result =
            if code_ == "" then [] else
                [ Bucket "audit-results" codeKey (label_ ++ " · " ++ controlKey ++ " v" ++ version) 1 resultOrder ]
    in
    Bucket "audit-days" day day 1 0 :: Bucket "audit-controls" controlKey controlKey 1 0 :: (result ++ followups)


selectionKey : String -> String -> String
selectionKey chartId key =
    chartId ++ "|" ++ key


selectBucket : String -> String -> Maybe String
selectBucket chartId key =
    if List.any (\( known, _ ) -> known == chartId) chartSpecs then
        Just (selectionKey chartId key)
    else
        Nothing


charts : List Row -> List ( String, List Bar )
charts rows =
    let
        accumulate bucket =
            Dict.update ( bucket.chart, bucket.key )
                (\existing -> Just { bucket | count = bucket.count + (Maybe.map .count existing |> Maybe.withDefault 0) })

        grouped = List.concatMap buckets rows |> List.foldl accumulate Dict.empty |> Dict.values

        series ( chartId, _ ) =
            ( chartId
            , grouped
                |> List.filter (\bucket -> bucket.chart == chartId)
                |> List.sortBy (\bucket -> ( bucket.order, bucket.label ))
                |> List.map (\bucket -> Bar bucket.key bucket.label (Just bucket.count)
                    (countLabel bucket.count (if chartId == "audit-followups" then "finding" else "run")))
            )
    in
    List.map series chartSpecs


listView : Maybe String -> (String -> msg) -> msg -> (String -> Html msg) -> Document -> Html msg
listView selected openRun clear chart doc =
    let
        visible = case selected of
            Nothing -> doc.rows
            Just key -> List.filter (buckets >> List.any (\bucket -> key == selectionKey bucket.chart bucket.key)) doc.rows
        series = charts doc.rows |> Dict.fromList
        graph ( chartId, title ) =
            section [ class "panel audit-chart-panel" ]
                [ div [ class "panel-heading" ] [ h2 [] [ text title ] ]
                , if List.isEmpty (Dict.get chartId series |> Maybe.withDefault []) then
                    p [ class "panel-empty" ] [ text (if chartId == "audit-results" then "No categorical results on this page." else "No observations on this page.") ]
                  else chart chartId
                ]
        total = String.fromFloat (Wire.number "total" doc.root)
    in
    div []
        [ p [ class "audit-cohort" ]
            [ text (total ++ " recorded executions · Coverage unknown")
            , small [] [ text ("Charts show " ++ String.fromInt (List.length doc.rows) ++ " runs on this page. Select a bar to inspect its runs.") ]
            ]
        , div [ class "metric-grid" ] (List.map graph chartSpecs)
        , section [ class "panel" ]
            ([ div [ class "panel-heading" ]
                [ h2 [ id "audit-executions", attribute "tabindex" "-1" ] [ text (if selected == Nothing then "Recorded executions" else "Executions in selected bar") ]
                , if selected == Nothing then text "" else button [ onClick clear ] [ text "Clear bar selection" ]
                ]
            ] ++ (if List.isEmpty visible then [ p [ class "panel-empty" ] [ text "No executions match these filters." ] ] else List.map (runRow openRun) visible))
        ]


runRow : (String -> msg) -> Row -> Html msg
runRow openRun row =
    let
        body = row.body
        control = Wire.field "control" body
        disposition = Wire.str "disposition" body
        resultLabel = Wire.str "result_label" body
        subject = subjectText (Wire.field "subject" body)
    in
    button [ class "record-row audit-run", onClick (openRun row.id) ]
        [ span [ class "audit-row-main" ]
            [ code [] [ text row.id ]
            , small [ class "muted" ] [ text (Wire.str "key" control ++ " v" ++ String.fromFloat (Wire.number "version" control) ++ " · " ++ Wire.str "occurred_at" body) ]
            , small [] [ text subject ]
            ]
        , span [ class "audit-row-state" ]
            [ text (caption (Wire.str "status" body))
            , small [ class "muted" ] [ text ((if resultLabel == "" then "" else resultLabel ++ " · ") ++ countLabel (Wire.number "open_findings" body) "open finding") ]
            , if disposition == "" then text "" else small [] [ text (caption disposition) ]
            ]
        ]


recordView : String -> String -> Document -> Html msg
recordView back runId doc =
    let
        definition = Wire.field "definition" doc.root
        event = definition |> Wire.field "input" |> Wire.field "event"
        policy = Wire.field "definition" event
        control = Wire.field "control" event
    in
    div []
        [ div [ class "audit-detail-title" ]
            [ div [] [ h2 [] [ text "Audit run" ], code [] [ text runId ] ]
            , a [ href back ] [ text "All audit runs" ]
            ]
        , section [ class "panel audit-event" ]
            [ h3 [] [ text (Wire.str "key" control ++ " · Version " ++ String.fromFloat (Wire.number "version" control)) ]
            , facts policy
                [ ( "owner", "Owner" ), ( "objective", "Objective" ), ( "scope", "Scope" ), ( "trigger", "Trigger or cadence" )
                , ( "procedure", "Procedure" ), ( "effective_at", "Effective from" ), ( "reason", "Version rationale" )
                ]
            , named "Framework references" (String.join ", " (Wire.strings "framework_references" policy))
            , named "Definition receipt" (Wire.str "recorded_at" definition)
            ]
        , section [ class "panel" ]
            (div [ class "panel-heading" ] [ h2 [] [ text "Run and follow-up history" ] ]
                :: List.map eventView (Wire.list "records" doc.root))
        ]


eventView : D.Value -> Html msg
eventView record =
    let
        entry = Wire.field "input" record
        event = Wire.field "event" entry
        actor = Wire.field "actor" record
        kind = Wire.str "kind" event
        data = case kind of
            "run" ->
                let run = Wire.field "run" event in
                [ named "Subject" (subjectText (Wire.field "subject" run))
                , facts run [ ( "executor", "Executor" ), ( "retry_of", "Retry of" ) ]
                , named "Duration (ms)" (D.decodeValue (D.field "duration_ms" D.int) run |> Result.map String.fromInt |> Result.withDefault "")
                , resultView (Wire.field "result" run)
                ]
            "correct_run" -> [ facts event [ ( "previous", "Replaces" ), ( "reason", "Correction rationale" ) ], resultView (Wire.field "result" event) ]
            "disposition" -> [ facts event [ ( "finding_id", "Finding" ), ( "previous", "Prior disposition" ), ( "disposition", "Disposition" ), ( "reason", "Reason" ) ], evidence event ]
            "verification" ->
                [ facts event [ ( "disposition_id", "Verifies disposition" ), ( "verifier", "Verifier (caller reported)" )
                  , ( "procedure", "Verification procedure" ), ( "outcome", "Verification outcome" ), ( "reason", "Reason" ) ]
                , evidence event ]
            _ -> []
    in
    details [ class "audit-event audit-history", attribute "open" "" ]
        (summary [] [ text (caption kind), code [] [ text (Wire.str "id" entry) ] ]
        :: named "Submitted by" (Wire.str "agent" actor ++ " · " ++ Wire.str "session_id" actor)
        :: facts entry [ ( "occurred_at", "Occurrence (caller reported)" ) ]
        :: named "Nexus receipt" (Wire.str "recorded_at" record)
        :: data)


resultView : D.Value -> Html msg
resultView result =
    let
        output = Wire.field "output" result
        findings = Wire.list "findings" result
        passes = Wire.list "review_passes" result
    in
    div []
        [ facts result [ ( "status", "Execution status" ), ( "summary", "Summary" ) ]
        , named "Result" (Wire.str "value" output)
        , evidence result
        , if List.isEmpty passes then text "" else
            div [ class "audit-nested" ] (h3 [] [ text "Review passes" ] :: List.map
                (\pass -> facts pass [ ( "perspective", "Perspective" ), ( "provider", "Provider" ), ( "model", "Model" )
                    , ( "outcome", "Outcome" ), ( "summary", "Summary" ), ( "instructions_sha256", "Instruction SHA-256" ) ]) passes)
        , if List.isEmpty findings then text "" else
            div [ class "audit-nested" ] (h3 [] [ text "Findings" ] :: List.map
                (\finding -> div [] [ facts finding [ ( "id", "Finding" ), ( "category", "Category" ), ( "description", "Description" )
                    , ( "recommendation", "Recommendation" ) ], evidence finding ]) findings)
        ]


subjectText : D.Value -> String
subjectText subject =
    case Wire.str "kind" subject of
        "external" -> Wire.str "source" subject ++ " · " ++ Wire.str "id" subject
        "git" -> revisionText (Wire.field "revision" subject)
        "staged_diff" ->
            let
                digest = Wire.str "sha256" subject
                base = Wire.str "commit" (Wire.field "base" subject)
            in
            Wire.str "repository" subject ++ (if base == "" then "" else " · Base " ++ base)
                ++ (if digest == "" then " · Staged diff unavailable" else " · Staged diff SHA-256 " ++ digest)
        _ -> "Subject unavailable"


revisionText : D.Value -> String
revisionText revision =
    Wire.str "repository" revision ++ " · " ++ Wire.str "commit" revision


evidence : D.Value -> Html msg
evidence container =
    let
        items = Wire.list "evidence" container
        item ref =
            let
                subject = Wire.field "subject" ref
                uri = Wire.str "uri" ref
                backing = case Wire.str "kind" ref of
                    "git" -> text (revisionText (Wire.field "revision" ref))
                    "work" -> text ("Work " ++ Wire.str "scope" subject ++ " " ++ Wire.str "id" subject)
                    "delivery" -> text (caption (Wire.str "kind" subject) ++ " " ++ Wire.str "id" subject)
                    "stored_blob" -> text ("Verified Nexus blob · SHA-256 " ++ Wire.str "digest" ref)
                    "external" -> if String.startsWith "https://" uri then a [ href uri, target "_blank", attribute "rel" "noopener noreferrer" ] [ text uri ] else text uri
                    _ -> text ("Unavailable · " ++ Wire.str "reason" ref)
            in
            p [] [ backing, text (if Wire.str "description" ref == "" then "" else " · " ++ Wire.str "description" ref) ]
    in
    if List.isEmpty items then text "" else div [ class "audit-nested" ] (h3 [] [ text "Evidence references" ] :: p [ class "muted" ] [ text "References are caller-reported unless marked verified." ] :: List.map item items)


named : String -> String -> Html msg
named title content =
    if content == "" then text "" else dl [ class "audit-facts" ] [ dt [] [ text title ], dd [] [ text content ] ]


facts : D.Value -> List ( String, String ) -> Html msg
facts data labels =
    dl [ class "audit-facts" ]
        (List.concatMap (\( key, title ) ->
            let content = Wire.str key data in
            if content == "" then [] else [ dt [] [ text title ], dd [] [ text content ] ]) labels)


caption : String -> String
caption key =
    case key of
        "unrecorded" -> "No disposition recorded"
        "risk_accepted" -> "Risk accepted"
        "correct_run" -> "Run correction"
        _ -> String.toUpper (String.left 1 key) ++ String.replace "_" " " (String.dropLeft 1 key)


countLabel : Float -> String -> String
countLabel count noun =
    String.fromFloat count ++ " " ++ noun ++ (if count == 1 then "" else "s")
