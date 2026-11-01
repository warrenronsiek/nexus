-- @feature observability-ui
-- @spec docs/features/observability-ui.md
-- @entrypoint main
-- @boundary browser-api: all domain data is decoded by Wire; credentials exist only in Elm form state until login completes.
port module Main exposing (main)

import Browser
import Browser.Navigation as Nav
import Dict exposing (Dict)
import Html exposing (Html, a, aside, button, code, dd, details, div, dl, dt, footer, form, h1, h2, h3, header, input, label, main_, nav, option, p, pre, section, select, small, span, summary, text)
import Html.Attributes exposing (attribute, autocomplete, class, disabled, href, id, name, placeholder, selected, type_, value)
import Html.Events exposing (on, onClick, onInput, onSubmit)
import Http
import Json.Decode as D
import Json.Encode as E
import State
import Time
import Url exposing (Url)
import Wire exposing (Document, Row)


port charts : E.Value -> Cmd msg


port chartSelection : (D.Value -> msg) -> Sub msg


type Screen
    = Coordination
    | Work
    | Evals
    | Tools
    | Skills
    | Concerns
    | Papercuts
    | Sos
    | Memory


type alias Query =
    { kind : String, fields : List ( String, E.Value ) }


type alias Model =
    { key : Nav.Key
    , screen : Screen
    , authenticated : Maybe Bool
    , username : String
    , password : String
    , loginError : Maybe String
    , project : Maybe String
    , projects : List Row
    , projectsPartial : Bool
    , projectDraft : String
    , data : State.Remote Document
    , supplemental : Dict String Document
    , query : Query
    , trail : List Query
    , selected : Maybe Row
    , metric : Int
    , zoom : Float
    , filters : Dict String String
    , mode : String
    , durationMode : String
    , trendKind : String
    }


type Msg
    = UrlRequested Browser.UrlRequest
    | UrlChanged Url
    | Username String
    | Password String
    | Login
    | Session (Result Http.Error String)
    | LoggedIn (Result Http.Error String)
    | Logout
    | LoggedOut (Result Http.Error String)
    | Projects (Result Http.Error String)
    | ChangeProject String
    | ProjectDraft String
    | Navigate Query
    | Back
    | Refresh
    | Tick Time.Posix
    | Received Int String Bool (Result Http.Error String)
    | Open Row
    | SelectNode String
    | SelectMetric String
    | ChartSelected D.Value
    | Zoom Float
    | Filter String String
    | ApplyFilters
    | Mode String
    | Keyboard String
    | More
    | DurationMode String
    | TrendKind String


main : Program () Model Msg
main =
    Browser.application { init = init, view = view, update = update, subscriptions = subscriptions, onUrlRequest = UrlRequested, onUrlChange = UrlChanged }


init : () -> Url -> Nav.Key -> ( Model, Cmd Msg )
init _ url key =
    ( { key = key, screen = route url, authenticated = Nothing, username = "", password = "", loginError = Nothing, project = Nothing, projects = [], projectsPartial = False, projectDraft = "", data = State.empty, supplemental = Dict.empty, query = primary (route url), trail = [], selected = Nothing, metric = 0, zoom = 1, filters = Dict.empty, mode = "", durationMode = "average", trendKind = "" }
    , http "GET" "session" Http.emptyBody Session
    )


screens : List Screen
screens =
    [ Coordination, Work, Evals, Tools, Skills, Concerns, Papercuts, Sos, Memory ]


slug : Screen -> String
slug screen =
    case screen of
        Coordination -> "coordination"
        Work -> "work"
        Evals -> "evals"
        Tools -> "tools"
        Skills -> "skills"
        Concerns -> "concerns"
        Papercuts -> "papercuts"
        Sos -> "sos"
        Memory -> "memory"


title : Screen -> String
title screen =
    case screen of
        Coordination -> "Coordination"
        Work -> "Work history"
        Evals -> "Evaluations"
        Tools -> "Tool intelligence"
        Skills -> "Skills"
        Concerns -> "Code concerns"
        Papercuts -> "Papercuts"
        Sos -> "SOS"
        Memory -> "Agent memory"


intro : Screen -> String
intro screen =
    case screen of
        Coordination -> "Live work, shared paths, and the signals that keep agents in sync."
        Work -> "Follow the time. Understand every session and the work nested inside it."
        Evals -> "What was tried, how it was measured, and why the evaluation changed."
        Tools -> "How agents interface with the world — and where a better tool is missing."
        Skills -> "The guidance agents actually use, across the last seven days."
        Concerns -> "Recurring code problems, their affected areas, and the remedies already tried."
        Papercuts -> "Development friction reported by agents across sessions."
        Sos -> "Unusual situations encountered by agents inside running applications."
        Memory -> "Durable knowledge, its sources, and the health of its summaries."


route : Url -> Screen
route url =
    List.filter (\screen -> String.endsWith ("/" ++ slug screen) url.path) screens |> List.head |> Maybe.withDefault Coordination


query : String -> Query
query kind =
    Query kind []


primary : Screen -> Query
primary screen =
    query <|
        case screen of
            Coordination -> "coordination"
            Work -> "work_dashboard"
            Evals -> "eval_dashboard"
            Tools -> "analytics_summary"
            Skills -> "usage"
            Concerns -> "concern_dashboard"
            Papercuts -> "papercuts"
            Sos -> "sos_list"
            Memory -> "memory_context"


request : String -> E.Value -> (Result Http.Error String -> Msg) -> Cmd Msg
request path body receive =
    http "POST" path (Http.jsonBody body) receive


http : String -> String -> Http.Body -> (Result Http.Error String -> Msg) -> Cmd Msg
http method path body receive =
    Http.request
        { method = method
        , headers = []
        , url = "/ui/api/" ++ path
        , body = body
        , expect = expectRaw receive
        , timeout = Just 20000
        , tracker = Nothing
        }


expectRaw : (Result Http.Error String -> Msg) -> Http.Expect Msg
expectRaw receive =
    Http.expectStringResponse receive <|
        \response ->
            case response of
                Http.BadUrl_ url -> Err (Http.BadUrl url)
                Http.Timeout_ -> Err Http.Timeout
                Http.NetworkError_ -> Err Http.NetworkError
                Http.BadStatus_ metadata body ->
                    if metadata.statusCode == 401 then Err (Http.BadStatus 401) else Err (Http.BadBody (D.decodeString (D.field "error" D.string) body |> Result.withDefault ("Nexus returned HTTP " ++ String.fromInt metadata.statusCode ++ ".")))
                Http.GoodStatus_ _ body -> Ok body


read : Maybe String -> Query -> (Result Http.Error String -> Msg) -> Cmd Msg
read project wanted receive =
    request "read" (E.object [ ( "project_id", Maybe.map E.string project |> Maybe.withDefault E.null ), ( "query", E.object (( "kind", E.string wanted.kind ) :: wanted.fields) ) ]) receive


scoped : Screen -> Bool
scoped screen =
    not (List.member screen [ Coordination, Tools, Skills ])


load : Query -> Model -> ( Model, Cmd Msg )
load wanted model =
    let
        previous = model.data
        pending = State.begin (if wanted == model.query then previous else { previous | value = Nothing })
        next = { model | data = pending, query = wanted }
        extras = if wanted.kind == "memory_context" then [ query "memory_status" ] else []
    in
    if scoped model.screen && model.project == Nothing && not (wanted.kind == "memory_expand" || String.startsWith "memory_" wanted.kind && List.member ( "scope", E.string "global" ) wanted.fields) then
        ( { next | data = State.accept pending.generation (Err "Choose a project to inspect its history.") pending }, Cmd.none )

    else
        ( next, Cmd.batch (read model.project wanted (Received pending.generation wanted.kind True) :: List.map (\q -> read model.project q (Received pending.generation q.kind False)) extras) )


reset : Screen -> Model -> ( Model, Cmd Msg )
reset screen model =
    load (primary screen) { model | screen = screen, data = { value = Nothing, generation = model.data.generation, loading = False, error = Nothing }, supplemental = Dict.empty, trail = [], selected = Nothing, metric = 0, zoom = 1, filters = Dict.empty, mode = "" }


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        UrlRequested (Browser.Internal url) ->
            ( model, Nav.pushUrl model.key (Url.toString url) )

        UrlRequested (Browser.External url) ->
            ( model, Nav.load url )

        UrlChanged url ->
            reset (route url) model

        Username username ->
            ( { model | username = username }, Cmd.none )

        Password password ->
            ( { model | password = password }, Cmd.none )

        Login ->
            ( { model | loginError = Nothing }, request "login" (E.object [ ( "username", E.string model.username ), ( "password", E.string model.password ) ]) LoggedIn )

        Session result ->
            case result |> Result.mapError httpError |> Result.andThen sessionDecoder of
                Ok ( True, username ) ->
                    ( { model | authenticated = Just True, username = username }, read Nothing (query "projects") Projects )

                Ok _ ->
                    ( { model | authenticated = Just False }, Cmd.none )

                Err reason ->
                    ( { model | authenticated = Just False, loginError = Just reason }, Cmd.none )

        LoggedIn result ->
            case result |> Result.mapError httpError |> Result.andThen sessionDecoder of
                Ok ( True, username ) ->
                    ( { model | authenticated = Just True, username = username, password = "", loginError = Nothing }, read Nothing (query "projects") Projects )

                Err reason ->
                    ( { model | authenticated = Just False, password = "", loginError = Just reason }, Cmd.none )

                Ok _ ->
                    ( { model | authenticated = Just False, password = "", loginError = Just "Sign-in failed. Check your configured credentials." }, Cmd.none )

        Logout ->
            ( model, request "logout" (E.object []) LoggedOut )

        LoggedOut result ->
            case result of
                Ok _ ->
                    ( { model | authenticated = Just False, password = "", data = State.empty, supplemental = Dict.empty, selected = Nothing }, Cmd.none )

                Err error ->
                    ( { model | loginError = Just (httpError error) }, Cmd.none )

        Projects result ->
            case result |> Result.mapError httpError |> Result.andThen (Wire.decode "projects") of
                Ok doc ->
                    load (primary model.screen) { model | projects = doc.rows, projectsPartial = D.decodeValue (D.field "truncated" D.bool) doc.root |> Result.withDefault False, project = model.project |> orElse (List.head doc.rows |> Maybe.map .id) }

                Err reason ->
                    ( { model | data = { value = Nothing, generation = model.data.generation + 1, error = Just reason, loading = False } }, Cmd.none )

        ChangeProject project ->
            reset model.screen { model | project = if project == "" then Nothing else Just project, projectDraft = "" }

        ProjectDraft project ->
            ( { model | projectDraft = project }, Cmd.none )

        Navigate wanted ->
            load wanted { model | trail = model.query :: model.trail, selected = Nothing }

        Back ->
            case model.trail of
                previous :: rest ->
                    load previous { model | trail = rest, selected = Nothing }

                [] ->
                    reset model.screen model

        Refresh ->
            load model.query model

        Tick _ ->
            if model.authenticated == Just True && not model.data.loading && model.selected == Nothing then
                load model.query model

            else
                ( model, Cmd.none )

        Received generation kind isPrimary result ->
            if generation /= model.data.generation then
                ( model, Cmd.none )

            else
                case result of
                    Err (Http.BadStatus 401) ->
                        ( { model | authenticated = Just False, data = State.empty, supplemental = Dict.empty, selected = Nothing, password = "", loginError = Just "Your session expired. Sign in to continue." }, Cmd.none )

                    _ ->
                        let
                            decoded = result |> Result.mapError httpError |> Result.andThen (Wire.decode kind)
                            updated = if isPrimary then { model | data = State.accept generation decoded model.data } else { model | supplemental = decoded |> Result.map (\doc -> Dict.insert kind doc model.supplemental) |> Result.withDefault model.supplemental }
                        in
                        ( updated, draw updated )

        Open row ->
            open row model

        SelectNode nodeId ->
            let
                rows = model.data.value |> Maybe.map waterfallRows |> Maybe.withDefault []
                picked = List.filter (\row -> row.id == nodeId) rows |> List.head
                next = { model | selected = picked }
            in
            ( next, draw next )

        SelectMetric index ->
            let
                next = { model | metric = String.toInt index |> Maybe.withDefault 0, selected = Nothing }
            in
            ( next, draw next )

        ChartSelected event ->
            case D.decodeValue (D.map2 Tuple.pair (D.field "chart" D.string) (D.field "id" D.string)) event of
                Ok ( "work-waterfall", nodeId ) ->
                    update (SelectNode nodeId) model

                Ok ( chartId, rowId ) ->
                    if chartId == "metric-trend" then
                        ( { model | selected = selectedMetric model |> Maybe.andThen (\m -> List.filter (\pt -> pt.id == rowId) m.points |> List.head) |> Maybe.map (\pt -> Row pt.id pt.label pt.exact "eval_record" (E.object [ ( "exact_value", E.string pt.exact ), ( "occurred_at", E.string pt.at ) ])) }, Cmd.none )

                    else
                        model.data.value |> Maybe.andThen (\doc -> List.filter (\row -> row.id == rowId) doc.rows |> List.head) |> Maybe.map (\row -> open row model) |> Maybe.withDefault ( model, Cmd.none )

                Err _ ->
                    ( model, Cmd.none )

        Zoom factor ->
            let
                next = { model | zoom = clamp 0.5 24 (model.zoom * factor) }
            in
            ( next, draw next )

        Filter key value_ ->
            ( { model | filters = Dict.insert key value_ model.filters }, Cmd.none )

        ApplyFilters ->
            applyFilters model

        Mode mode ->
            load (modeQuery model.screen mode) { model | mode = mode, trail = [], selected = Nothing }

        Keyboard key ->
            keyboard key model

        More ->
            more model

        DurationMode mode ->
            let next = { model | durationMode = mode } in
            ( next, draw next )

        TrendKind kind ->
            let next = { model | trendKind = kind } in
            ( next, draw next )


sessionDecoder : String -> Result String ( Bool, String )
sessionDecoder raw =
    D.decodeString (D.map2 Tuple.pair (D.field "authenticated" D.bool) (D.oneOf [ D.field "username" D.string, D.succeed "" ])) raw |> Result.mapError D.errorToString


httpError : Http.Error -> String
httpError error =
    case error of
        Http.BadUrl _ -> "The interface endpoint is invalid."
        Http.Timeout -> "The read timed out. Previous evidence is retained; refresh to retry."
        Http.NetworkError -> "Nexus is unreachable. Previous evidence is retained."
        Http.BadStatus status -> "Nexus returned HTTP " ++ String.fromInt status ++ ". Refresh to retry."
        Http.BadBody reason -> reason


orElse : Maybe a -> Maybe a -> Maybe a
orElse fallback preferred =
    case preferred of
        Just _ -> preferred
        Nothing -> fallback


with : String -> String -> Query -> Query
with key value_ wanted =
    { wanted | fields = wanted.fields ++ [ ( key, E.string value_ ) ] }


subject : String -> String -> E.Value
subject scope id_ =
    E.object [ ( "scope", E.string scope ), ( "id", E.string id_ ) ]


open : Row -> Model -> ( Model, Cmd Msg )
open row model =
    let
        follow wanted = load wanted { model | trail = model.query :: model.trail, selected = Nothing }
    in
    case row.kind of
        "eval_group" -> follow (query "eval_dashboard" |> with "group_id" row.id)
        "eval_record" -> follow (query "eval_get" |> with "id" row.id)
        "session" ->
            if model.screen == Coordination then
                ( { model | selected = Just row }, Cmd.none )
            else
                follow (query "work_dashboard" |> with "work_session_id" row.id)
        "activity" -> follow (Query "work_get" [ ( "subject", subject "activity" row.id ) ])
        "concern_area" -> follow (Query "concern_list" [ ( "filter", E.object [ ( "area_id", E.string row.id ) ] ) ])
        "items" -> follow (query "concern_get" |> with "concern_id" row.id)
        "issues" -> follow (query "sos_get" |> with "issue_id" row.id)
        "capability" -> follow (query "analytics_examples" |> with "resource_key" row.id)
        "opportunity" -> follow (query "analytics_examples" |> with "category_id" row.id)
        "activity_category" -> follow (query "analytics_examples" |> with "category_id" row.id)
        "summary" -> follow (query "memory_expand" |> with "summary_id" row.id)
        _ ->
            if model.screen == Tools && model.query.kind == "analytics_summary" then
                follow (query "analytics_examples" |> with "category_id" row.id)
            else
                ( { model | selected = Just row }, Cmd.none )


modeQuery : Screen -> String -> Query
modeQuery screen mode =
    query <|
        case screen of
            Coordination -> if mode == "" then "coordination" else "coordination_" ++ mode
            Work -> if mode == "activities" then "work_activities" else if mode == "records" then "work_list" else "work_dashboard"
            Tools -> if mode == "capabilities" then "analytics_catalog" else if mode == "opportunities" then "analytics_opportunities" else if mode == "legacy" then "usage" else "analytics_summary"
            Evals -> if mode == "records" then "eval_list" else "eval_dashboard"
            Concerns -> if mode == "all" then "concern_list" else "concern_dashboard"
            _ -> (primary screen).kind


applyFilters : Model -> ( Model, Cmd Msg )
applyFilters model =
    let
        fields = Dict.toList model.filters |> List.filter (\( _, v ) -> v /= "") |> List.map (Tuple.mapSecond E.string)
        kind = case model.screen of
            Work -> if model.mode == "activities" || (Dict.get "kind" model.filters |> Maybe.withDefault "") /= "" then "work_activities" else "work_list"
            Evals -> if model.mode == "records" || (Dict.get "kind" model.filters |> Maybe.withDefault "") /= "" then "eval_list" else "eval_dashboard"
            Concerns -> if model.mode == "all" then "concern_list" else "concern_dashboard"
            Memory -> if (Dict.get "regex" model.filters |> Maybe.withDefault "") == "" then "memory_context" else "memory_search"
            _ -> model.query.kind
        wanted = if model.screen == Memory then Query kind fields else if kind == "eval_dashboard" then query kind else Query kind [ ( "filter", E.object fields ) ]
    in
    load wanted { model | selected = Nothing, trail = [] }


more : Model -> ( Model, Cmd Msg )
more model =
    case model.data.value of
        Nothing -> ( model, Cmd.none )
        Just doc ->
            let
                next = case model.query.kind of
                    "papercuts" -> List.reverse doc.rows |> List.head |> Maybe.map (\row -> E.object [ ( "updated_at", E.string (Wire.str "updated_at" row.body) ), ( "id", E.string row.id ) ])
                    "analytics_catalog" -> D.decodeValue (D.field "next_offset" (D.nullable D.int)) doc.root |> Result.withDefault Nothing |> Maybe.map E.int
                    "analytics_examples" -> D.decodeValue (D.field "next_offset" (D.nullable D.int)) doc.root |> Result.withDefault Nothing |> Maybe.map E.int
                    _ -> doc.next
                wanted cursor =
                    if List.member model.query.kind [ "work_list", "work_activities", "eval_list", "concern_list", "concern_dashboard" ] then
                        let
                            active = List.filter (\( key, _ ) -> key == "filter") model.query.fields |> List.head |> Maybe.map Tuple.second |> Maybe.andThen (D.decodeValue (D.keyValuePairs D.value) >> Result.toMaybe) |> Maybe.withDefault []
                        in
                        Query model.query.kind [ ( "filter", E.object (( "after", cursor ) :: List.filter (\( key, _ ) -> key /= "after") active) ) ]
                    else
                        { kind = model.query.kind, fields = List.filter (\( k, _ ) -> k /= "after" && k /= "offset") model.query.fields ++ [ ( if String.startsWith "analytics_" model.query.kind then "offset" else "after", cursor ) ] }
            in
            next |> Maybe.map (\cursor -> load (wanted cursor) { model | trail = model.query :: model.trail, selected = Nothing }) |> Maybe.withDefault ( model, Cmd.none )


waterfallRows : Document -> List Row
waterfallRows doc =
    let
        root = Wire.field "selected_session" doc.root
        rootId = Wire.str "id" (Wire.field "subject" root)
        session = if rootId == "" then [] else [ Row rootId (Wire.str "title" root) "Session" "session" root ]
    in
    session ++ List.map (\node -> Row node.id node.label node.kind "activity" node.body) doc.intervals


keyboard : String -> Model -> ( Model, Cmd Msg )
keyboard key model =
    if model.screen /= Work || model.query.kind /= "work_dashboard" then
        ( model, Cmd.none )
    else
        let
            rows = model.data.value |> Maybe.map waterfallRows |> Maybe.withDefault []
            selectedId = model.selected |> Maybe.map .id |> Maybe.withDefault (List.head rows |> Maybe.map .id |> Maybe.withDefault "")
            index = rows |> List.indexedMap Tuple.pair |> List.filter (\( _, row ) -> row.id == selectedId) |> List.head |> Maybe.map Tuple.first |> Maybe.withDefault 0
            parent = model.selected |> Maybe.map (\row -> Wire.str "parent_activity_id" row.body) |> Maybe.withDefault ""
            picked = case key of
                "ArrowDown" -> List.drop (index + 1) rows |> List.head
                "ArrowUp" -> List.drop (max 0 (index - 1)) rows |> List.head
                "ArrowLeft" -> List.filter (\row -> row.id == parent) rows |> List.head |> orElse (List.head rows)
                "ArrowRight" -> List.filter (\row -> Wire.str "parent_activity_id" row.body == selectedId || index == 0 && row.kind == "activity" && Wire.str "parent_activity_id" row.body == "") rows |> List.head
                _ -> Nothing
        in
        picked |> Maybe.map (\row -> update (SelectNode row.id) model) |> Maybe.withDefault ( model, Cmd.none )


subscriptions : Model -> Sub Msg
subscriptions model =
    Sub.batch [ chartSelection ChartSelected, if model.authenticated == Just True then Time.every 30000 Tick else Sub.none ]


selectedMetric : Model -> Maybe Wire.Metric
selectedMetric model =
    model.data.value |> Maybe.andThen (\doc -> List.drop model.metric doc.metrics |> List.head)


draw : Model -> Cmd Msg
draw model =
    case model.data.value of
        Nothing -> Cmd.none
        Just doc ->
            let
                metricKey = if model.durationMode == "total" then "total_seconds" else "average_seconds"
                workBars = Wire.list "activity_kinds" doc.root |> List.map (\row -> Wire.Bar (Wire.str "kind" row) (Wire.str "kind" row) (if Wire.number "known_count" row == 0 then Nothing else Just (Wire.number metricKey row)) ((if Wire.number "known_count" row == 0 then "Unknown" else duration (Wire.number metricKey row)) ++ " · " ++ countText "known_count" row ++ "/" ++ countText "count" row ++ " timed"))
                visibleBars = if model.query.kind == "usage" then doc.rows |> List.filter (\row -> row.kind == (if model.screen == Skills then "skill" else "tool")) |> List.map (\row -> Wire.Bar row.id row.title (Just (Wire.number "count" row.body)) row.subtitle) else if model.query.kind == "work_dashboard" then workBars else doc.bars
                barChart = if List.isEmpty visibleBars then [] else [ E.object [ ( "id", E.string "summary-bars" ), ( "type", E.string "bars" ), ( "bars", E.list (\bar -> E.object [ ( "id", E.string bar.id ), ( "label", E.string bar.label ), ( "value", bar.value |> Maybe.map E.float |> Maybe.withDefault E.null ), ( "detail", E.string bar.detail ) ]) visibleBars ) ] ]
                workDays = if model.trendKind == "" then Wire.list "daily" doc.root else Wire.list "activity_daily" doc.root |> List.filter (\row -> Wire.str "kind" row == model.trendKind)
                workPoints = workDays |> List.map (\row -> Wire.Point (Wire.str "date" row) (Wire.str "date" row) (Wire.str "date" row) (if Wire.number "known_count" row == 0 then Nothing else Just (Wire.number metricKey row)) "")
                visiblePoints = if model.query.kind == "work_dashboard" then workPoints else doc.points
                trendChart = if List.isEmpty visiblePoints then [] else [ encodeTrend "summary-trend" visiblePoints ]
                metricChart = selectedMetric model |> Maybe.map (\metric -> [ encodeTrend "metric-trend" metric.points ]) |> Maybe.withDefault []
                root = Wire.field "selected_session" doc.root
                rootId = Wire.str "id" (Wire.field "subject" root)
                rootNode = { id = rootId, parent = Nothing, label = Wire.str "title" root, kind = "work.session", start = maybeText (Wire.field "started_at" root), end = maybeText (Wire.field "finished_at" root), queued = maybeText (Wire.field "queued_at" root), body = root }
                graph = if rootId == "" then [] else [ E.object [ ( "id", E.string "work-waterfall" ), ( "type", E.string "waterfall" ), ( "selected", model.selected |> Maybe.map (.id >> E.string) |> Maybe.withDefault E.null ), ( "zoom", E.float model.zoom ), ( "spans", E.list (encodeInterval rootId) (rootNode :: doc.intervals) ) ] ]
            in
            charts (E.list identity (barChart ++ trendChart ++ metricChart ++ graph))


maybeText : D.Value -> Maybe String
maybeText value_ =
    D.decodeValue (D.nullable D.string) value_ |> Result.withDefault Nothing


encodeInterval : String -> Wire.Interval -> E.Value
encodeInterval rootId node =
    E.object [ ( "id", E.string node.id ), ( "label", E.string node.label ), ( "kind", E.string node.kind ), ( "parent", (if node.id == rootId then Nothing else node.parent |> orElse (Just rootId)) |> Maybe.map E.string |> Maybe.withDefault E.null ), ( "start", encodeTime node.start ), ( "end", encodeTime node.end ), ( "queued", encodeTime node.queued ) ]


encodeTime : Maybe String -> E.Value
encodeTime value_ =
    -- JS Date parsing is confined to chart positioning. Source timestamps remain literal in Elm.
    value_ |> Maybe.map E.string |> Maybe.withDefault E.null


encodeTrend : String -> List Wire.Point -> E.Value
encodeTrend chartId points =
    E.object [ ( "id", E.string chartId ), ( "type", E.string "trend" ), ( "points", E.list (\pt -> E.object [ ( "id", E.string pt.id ), ( "label", E.string pt.label ), ( "at", E.string pt.at ), ( "value", pt.value |> Maybe.map E.float |> Maybe.withDefault E.null ) ]) points ) ]


view : Model -> Browser.Document Msg
view model =
    { title = "Nexus · " ++ title model.screen
    , body =
        [ case model.authenticated of
            Nothing -> div [ class "connection" ] [ div [ class "brand-mark" ] [ text "N" ], h2 [] [ text "Connecting to Nexus" ], p [] [ text "Opening your observation workspace…" ] ]
            Just False -> loginView model
            Just True -> workspace model
        ]
    }


loginView : Model -> Html Msg
loginView model =
    main_ [ class "login-shell" ]
        [ div [ class "login-story" ]
            [ div [ class "wordmark" ] [ span [ class "brand-mark" ] [ text "N" ], text "nexus" ]
            , div [] [ span [ class "eyebrow" ] [ text "AGENT INTELLIGENCE" ], h1 [] [ text "A clearer view of", Html.br [] [], text "the work behind", Html.br [] [], span [] [ text "the outcome." ] ], p [] [ text "One durable history of agent work, evaluations, and the evidence that connects them." ] ]
            , small [] [ text "Observe. Understand. Improve." ]
            ]
        , div [ class "login-side" ]
            [ form [ class "login-card", onSubmit Login ]
                [ span [ class "eyebrow" ] [ text "YOUR WORKSPACE" ], h2 [] [ text "Welcome to Nexus" ], p [ class "muted" ] [ text "Sign in with the credentials configured during setup." ]
                , label [] [ text "Username", input [ name "username", autocomplete True, attribute "autocomplete" "username", value model.username, onInput Username, attribute "required" "true" ] [] ]
                , label [] [ text "Password", input [ name "password", type_ "password", attribute "autocomplete" "current-password", value model.password, onInput Password, attribute "required" "true" ] [] ]
                , model.loginError |> Maybe.map (\reason -> div [ class "notice error", attribute "role" "alert" ] [ text reason ]) |> Maybe.withDefault (text "")
                , button [ class "primary", type_ "submit" ] [ text "Sign in →" ]
                , div [ class "login-note" ] [ span [ class "status-dot" ] [], text "Read-only access · Your evidence stays unchanged" ]
                ]
            ]
        ]


workspace : Model -> Html Msg
workspace model =
    div [ class "workspace" ]
        [ aside [ class "sidebar" ]
            [ a [ class "wordmark", href "/ui/coordination" ] [ span [ class "brand-mark" ] [ text "N" ], text "nexus", small [] [ text "OBSERVE" ] ]
            , div [ class "nav-caption" ] [ text "WORKSPACE" ]
            , nav [] (List.indexedMap (\index screen -> a [ href ("/ui/" ++ slug screen), class (if model.screen == screen then "nav-item active" else "nav-item"), attribute "aria-current" (if model.screen == screen then "page" else "false") ] [ span [ class "nav-symbol" ] [ text (List.drop index [ "◈", "⌁", "◬", "⬡", "◇", "◎", "◌", "⚑", "▤" ] |> List.head |> Maybe.withDefault "·") ], text (title screen) ]) screens)
            , div [ class "sidebar-bottom" ] [ div [ class "read-only" ] [ span [ class "status-dot" ] [], text "Read-only workspace" ], p [] [ text "Evidence, never assumptions." ] ]
            ]
        , div [ class "main-shell" ]
            [ header [ class "topbar" ]
                [ div [ class "breadcrumb" ] [ text "Workspace", span [] [ text "/" ], text (title model.screen) ]
                , div [ class "top-actions" ] [ span [ class "account" ] [ text model.username ], button [ onClick Logout, class "quiet" ] [ text "Sign out" ] ]
                ]
            , main_ [ class "content" ]
                [ div [ class "page-heading" ] [ div [] [ span [ class "eyebrow" ] [ text "NEXUS / OBSERVABILITY" ], h1 [] [ text (title model.screen) ], p [] [ text (intro model.screen) ] ], div [ class "live-label" ] [ span [ class "status-dot" ] [], text "Recorded evidence" ] ]
                , toolbar model
                , statusView model
                , case model.data.value of
                    Nothing -> div [ class "empty-state" ] [ div [ class "empty-glyph" ] [ text "◈" ], h2 [] [ text (if model.data.loading then "Loading observations" else "Ready when you are") ], p [] [ text (if model.data.loading then "Retrieving the latest retained evidence…" else "Select a project or refresh to retrieve evidence.") ] ]
                    Just doc -> documentView model doc
                ]
            , footer [ class "page-footer" ] [ text "Nexus · Durable agent evidence", span [] [ text "Read-only · Unknown and partial capture remain explicit" ] ]
            ]
        ]


toolbar : Model -> Html Msg
toolbar model =
    let
        current = Maybe.withDefault "" model.project
        catalog = List.map (\row -> choice current row.id (if row.subtitle == "" then row.id else row.subtitle)) model.projects
        custom = if current /= "" && not (List.any (\row -> row.id == current) model.projects) then [ choice current current current ] else []
    in
    div []
        [ div [ class "toolbar" ]
            [ div [ class "scope-control" ] [ span [ class "muted" ] [ text "Project" ], select [ onInput ChangeProject, attribute "aria-label" "Project scope" ] (choice current "" "All projects" :: custom ++ catalog) ]
            , div [ class "toolbar-right" ] [ if List.isEmpty model.trail then text "" else button [ onClick Back ] [ text "← Back" ], button [ onClick Refresh, disabled model.data.loading ] [ text (if model.data.loading then "Refreshing…" else "↻ Refresh") ] ]
            ]
        , if model.projectsPartial then
            div [ class "notice subtle project-catalog" ]
                [ p [] [ text "Project discovery is bounded. Enter a known project ID to inspect another project." ]
                , form [ onSubmit (ChangeProject model.projectDraft) ] [ input [ name "project_id", attribute "aria-label" "Another project ID", value model.projectDraft, onInput ProjectDraft, placeholder "Project ID", attribute "required" "true" ] [], button [ type_ "submit" ] [ text "Use project" ] ]
                ]
          else text ""
        ]


choice : String -> String -> String -> Html Msg
choice current key caption =
    option [ attribute "value" key, selected (current == key) ] [ text caption ]


statusView : Model -> Html Msg
statusView model =
    div [ attribute "aria-live" "polite" ]
        [ model.data.error |> Maybe.map (\reason -> div [ class "notice error" ] [ text reason, if model.data.value /= Nothing then span [] [ text " · Showing the last successful snapshot." ] else text "" ]) |> Maybe.withDefault (text "")
        , model.loginError |> Maybe.map (\reason -> div [ class "notice error" ] [ text reason ]) |> Maybe.withDefault (text "")
        ]


documentView : Model -> Document -> Html Msg
documentView model doc =
    div [ class "document" ]
        [ subnav model
        , filtersView model
        , coverage doc
        , if model.query.kind == "work_dashboard" then workView model doc else if model.query.kind == "eval_dashboard" then evalView model doc else standardView model doc
                , if List.isEmpty doc.rows then emptyRecords model doc else rowList model doc
        , if model.query.kind == "work_dashboard" then text "" else model.selected |> Maybe.map (selectedView model) |> Maybe.withDefault (text "")
        , if List.member model.query.kind [ "work_get", "work_observation", "eval_get", "concern_get", "sos_get", "memory_status" ] then evidenceView model doc else text ""
        , pagination model doc
        , details [ class "raw-evidence" ] [ summary [] [ text "Original response · exact immutable JSON" ], p [ class "muted" ] [ text "This is the unchanged response text. Numeric chart positions are display approximations; source values here retain their exact representation." ], pre [] [ code [] [ text doc.raw ] ] ]
        ]


emptyRecords : Model -> Document -> Html Msg
emptyRecords model doc =
    if List.member model.query.kind [ "eval_list", "work_list", "work_activities", "work_timeline", "analytics_catalog", "analytics_examples", "analytics_summary", "analytics_opportunities", "memory_context", "memory_expand", "memory_search", "papercuts", "sos_list", "concern_list", "concern_history" ] then
        panel "No recorded observations" "No records were returned for this scope and filter. Absence of reports does not establish complete capture or verified health." []
    else
        text ""


subnav : Model -> Html Msg
subnav model =
    let
        modes = case model.screen of
            Coordination -> [ ( "", "Overview" ), ( "sessions", "Sessions" ), ( "claims", "Claims" ), ( "conflicts", "Conflicts" ), ( "events", "Events" ) ]
            Work -> [ ( "", "Duration overview" ), ( "records", "Sessions" ), ( "activities", "Activity search" ) ]
            Evals -> [ ( "", "Evaluation sets" ), ( "records", "Evidence records" ) ]
            Tools -> [ ( "", "Activities" ), ( "capabilities", "Capabilities" ), ( "opportunities", "Opportunities" ), ( "legacy", "Legacy" ) ]
            Concerns -> [ ( "", "Problem areas" ), ( "all", "All concerns" ) ]
            Memory -> [ ( "", "Knowledge" ) ]
            _ -> []
    in
    div [ class "subnav" ] (List.map (\( mode, caption ) -> button [ class (if mode == model.mode then "active" else ""), onClick (Mode mode) ] [ text caption ]) modes)


filtersView : Model -> Html Msg
filtersView model =
    let
        fields = case model.screen of
            Work -> [ ( "search", "Search sessions / activities" ), ( "repository", "Repository" ), ( "host_session_id", "Host session" ), ( "kind", "Activity kind" ), ( "outcome", "Outcome" ), ( "environment", "Environment" ), ( "occurred_after", "From ISO timestamp" ), ( "occurred_before", "To ISO timestamp" ) ]
            Evals -> [ ( "search", "Search evidence" ), ( "repository", "Repository" ), ( "model", "Model / agent" ), ( "kind", "Record kind" ) ]
            Concerns -> [ ( "search", "Search concerns" ), ( "repository", "Repository" ), ( "status", "Status: open / resolved" ), ( "sort", "recent / work_sessions / returns" ), ( "occurred_after", "From ISO timestamp" ), ( "occurred_before", "To ISO timestamp" ) ]
            Memory -> [ ( "regex", "Search knowledge (regular expression)" ), ( "scope", "Scope: layered / project / global" ) ]
            _ -> []
    in
    if List.isEmpty fields then text "" else
        details [ class "filter-panel" ] [ summary [] [ text "⌕ Search and filters", if Dict.isEmpty model.filters then text "" else span [ class "badge" ] [ text (String.fromInt (Dict.size model.filters)) ] ], form [ onSubmit ApplyFilters ] (List.map (\( key, caption ) -> label [] [ text caption, input [ value (Dict.get key model.filters |> Maybe.withDefault ""), onInput (Filter key), placeholder caption ] [] ]) fields ++ [ button [ type_ "submit", class "primary" ] [ text "Apply filters" ] ]) ]


coverage : Document -> Html Msg
coverage doc =
    let
        paths = List.map List.singleton [ "truncated", "partial" ] ++ List.map (\key -> [ key, "truncated" ]) [ "sessions", "claims", "conflicts", "events" ] ++ List.map (\key -> [ "detail", key ]) [ "reports_truncated", "triage_attempts_truncated", "lifecycle_events_truncated" ] ++ [ [ "concern", "partial" ], [ "record", "truncated" ], [ "record", "lifecycle", "truncated" ] ]
        partial = List.any (\path -> D.decodeValue (D.at path D.bool) doc.root |> Result.withDefault False) paths
    in
    div []
        ((if partial then [ div [ class "notice" ] [ text "Partial history · The current response is bounded. Charts describe retained observations, not the entire population." ] ] else [])
            ++ List.map (\notice -> div [ class "notice subtle" ] [ text notice ]) doc.notices
        )


stat : String -> String -> String -> Html Msg
stat caption value_ note =
    div [ class "stat" ] [ div [ class "stat-label" ] [ text caption ], div [ class "stat-value" ] [ text value_ ], small [] [ text note ] ]


countText : String -> D.Value -> String
countText key root =
    String.fromFloat (Wire.number key root)


duration : Float -> String
duration seconds =
    if seconds >= 3600 then String.fromFloat (toFloat (round (seconds / 36)) / 100) ++ "h" else if seconds >= 60 then String.fromFloat (toFloat (round (seconds / 6)) / 10) ++ "m" else String.fromFloat (toFloat (round (seconds * 100)) / 100) ++ "s"


knownDuration : String -> D.Value -> String
knownDuration key root =
    D.decodeValue (D.field key D.float) root |> Result.map duration |> Result.withDefault "Unknown"


panel : String -> String -> List (Html Msg) -> Html Msg
panel heading caption body =
    section [ class "panel" ] ([ div [ class "panel-heading" ] [ div [] [ h2 [] [ text heading ], p [] [ text caption ] ] ] ] ++ body)


chart : String -> Html Msg
chart chartId =
    div [ id chartId, class "chart" ] []


workView : Model -> Document -> Html Msg
workView model doc =
    let
        stats = Wire.field "session_duration" doc.root
        root = Wire.field "selected_session" doc.root
        accounting = Wire.field "accounting" doc.root
        cover = Wire.field "coverage" accounting
        selectedSession = Wire.str "id" (Wire.field "subject" root) /= ""
    in
    if selectedSession then
        div []
            [ div [ class "stats-grid" ]
                [ stat "Session elapsed" (exactElapsed root) "Recorded elapsed evidence"
                , stat "Attributed wall" (knownDuration "seconds" (Wire.field "occupied_wall" accounting)) "Execution / wait union"
                , stat "Unobserved" (knownDuration "seconds" (Wire.field "unobserved" accounting)) "No invented attribution"
                , stat "Worker occupancy" (knownDuration "seconds" (Wire.field "worker_occupancy" accounting)) "Inclusive across executors; not effort"
                ]
            , panel (Wire.str "title" root) "Time flows downward. Children branch right at their source admission/start. Select any task to inspect its metadata below."
                [ div [ class "graph-controls" ] [ div [ class "legend" ] (List.map (\( css, caption ) -> span [ class css ] [ text ("● " ++ caption) ]) [ ( "agent", "Subagent" ), ( "test", "Test" ), ( "script", "Script" ), ( "review", "Review" ), ( "tool", "Tool / command" ), ( "session", "Session / other" ) ]), div [] [ button [ onClick (Zoom 0.7), attribute "aria-label" "Zoom out" ] [ text "−" ], button [ onClick (Zoom 1.4), attribute "aria-label" "Zoom in" ] [ text "+" ] ] ]
                , div [ class "waterfall-scroll", attribute "tabindex" "0", on "keydown" (D.field "key" D.string |> D.map Keyboard), attribute "aria-label" "Session task waterfall. Arrow keys select tasks, parents and children." ] [ chart "work-waterfall" ]
                , p [ class "chart-note" ] [ text "Solid span: known source boundaries · dashed span: unknown finish · dotted span: queue wait · arrow keys: task / parent / child · scroll to pan" ]
                ]
            , model.selected |> Maybe.map (selectedView model) |> Maybe.withDefault (panel "Task metadata" "Select a task in the waterfall to inspect its recorded boundaries, executor, parent and evidence." [])
            , panel "Accounting coverage" "Missing capture, roles, clocks and source inventory qualify these numbers." [ facts cover, div [ class "definitions" ] (List.map (\definition -> p [] [ text definition ]) (Wire.strings "definitions" accounting)) ]
            , div [ class "action-row" ] [ button [ onClick (Navigate (Query "work_timeline" [ ( "work_session_id", E.string (Wire.str "id" (Wire.field "subject" root)) ) ])) ] [ text "Browse ownership hierarchy" ], button [ onClick (Navigate (Query "work_get" [ ( "subject", Wire.field "subject" root ) ])) ] [ text "Session evidence" ] ]
            ]
    else
        div []
            [ div [ class "stats-grid" ]
                [ stat "Average session" (knownDuration "average_seconds" stats) "Known, non-conflicted elapsed durations"
                , stat "Total session elapsed" (if Wire.number "known_count" stats == 0 then "Unknown" else knownDuration "total_seconds" stats) "Inclusive session intervals; not effort"
                , stat "Sessions measured" (countText "known_count" stats ++ " / " ++ countText "count" stats) "Unknown values remain unknown"
                , stat "Activity kinds" (String.fromInt (List.length doc.bars)) "Recorded kinds, never guessed from titles"
                ]
            , div [ class "duration-controls" ]
                [ label [] [ text "Duration aggregation", select [ onInput DurationMode ] [ choice model.durationMode "average" "Average known duration", choice model.durationMode "total" "Total known duration" ] ]
                , label [] [ text "Time series", select [ onInput TrendKind ] (choice model.trendKind "" "Session duration" :: List.map (\bar -> choice model.trendKind bar.id (bar.label ++ " activities")) doc.bars) ]
                ]
            , panel "Duration over time" (model.durationMode ++ " known elapsed duration by UTC source day · seconds") [ chart "summary-trend" ]
            , panel "Where time goes" (model.durationMode ++ " elapsed duration by activity kind. Parent and child durations overlap.") [ chart "summary-bars" ]
            ]


evalView : Model -> Document -> Html Msg
evalView model doc =
    div []
        [ div [ class "stats-grid" ] [ stat "Recorded runs" (countText "run_count" doc.root) "Caller-owned execution", stat "Results" (countText "result_count" doc.root) "Current observations; corrections retained", stat "Evaluation sets" (String.fromInt (List.length doc.rows)) "Suite × target × repository × model composition", stat "Source records" (countText "scanned_records" doc.root) "Bounded historical projection" ]
        , if List.isEmpty doc.metrics then
            panel "Evaluation sets" "Select a set to explore the metric definitions, recorded results and historical evidence." []
          else
            div []
                [ div [ class "metric-select" ] [ label [] [ text "Metric definition / revision", select [ onInput SelectMetric ] (List.indexedMap (\index metric -> choice (String.fromInt model.metric) (String.fromInt index) (metric.label ++ " · " ++ metric.unit ++ " · " ++ metric.definition)) doc.metrics) ] ]
                , selectedMetric model |> Maybe.map (\metric -> panel metric.label (metric.unit ++ " · " ++ metric.description) [ chart "metric-trend", div [ class "metric-definition" ] [ h3 [] [ text "How this metric is calculated" ], p [] [ text metric.calculation ], code [] [ text metric.definition ], p [ class "chart-note" ] [ text "Numeric positions are display approximations. Metric meanings and revisions remain separate; Nexus assumes no preferred direction." ] ], div [ class "result-values" ] (List.reverse metric.points |> List.take 12 |> List.map (\pt -> button [ onClick (Open (Row pt.id pt.label pt.exact "eval_record" E.null)), class "result-value" ] [ small [] [ text pt.at ], strongText pt.exact ])) ]) |> Maybe.withDefault (text "")
                ]
        ]


strongText : String -> Html Msg
strongText value_ =
    Html.strong [] [ text value_ ]


standardView : Model -> Document -> Html Msg
standardView model doc =
    case model.screen of
        Coordination ->
            let
                counts = Wire.field "counts" doc.root
            in
            if model.query.kind /= "coordination" then text "" else
                div [ class "stats-grid" ] [ stat "Recent sessions" (countText "active_sessions" counts) "Live coordination liveness", stat "Live claims" (countText "active_claims" counts) "Advisory path ownership", stat "Open conflicts" (countText "open_conflicts" counts) "Live unresolved overlaps", stat "Recent events" (countText "events" counts) "Bounded operational history" ]

        Tools ->
            div []
                [ div [ class "stats-grid" ] [ stat "Observed calls" (countText "observed_calls" doc.root) "Invocations may contain several operations", stat "Identified operations" (countText "identified_actions" doc.root) "Semantic activity classification", stat "Unclassified" (countText "unclassified_actions" doc.root) "Coverage remains explicit", stat "Window" "7 days" "Rolling observation window" ]
                , if List.isEmpty doc.bars then text "" else panel "What agents are doing" "Observed operations by semantic category, including named tools and improvised implementations." [ chart "summary-bars" ]
                , if model.mode == "capabilities" then div [ class "notice subtle" ] [ text "Zero observed uses are meaningful only where inventory establishes availability. Offered and discoverable sessions are separate." ] else if model.mode == "opportunities" then div [ class "notice subtle" ] [ text "Recurring improvised work can suggest a missing capability. Classification describes inferred purpose; it does not prove every code branch executed." ] else text ""
                ]

        Skills ->
            panel "Skill usage" "Named skills across the rolling seven-day window. Counts reflect captured observations." [ chart "summary-bars", facts (Wire.field "capture_health" doc.root) ]

        Concerns ->
            let counts = Wire.field "counts" doc.root in
            if model.query.kind /= "concern_dashboard" then text "" else
                div []
                    [ div [ class "stats-grid" ] [ stat "Reported concerns" (countText "concerns" counts) "No reports does not establish code health", stat "Affected work sessions" (countText "work_sessions" counts) "Distinct retained session identities", stat "Source occurrences" (countText "known_source_occurrences" counts) "Known failure encounters", stat "Returns after resolution" (countText "returns_after_resolution" counts) "Potential recurring problem areas" ]
                    , panel "Problem areas" "Affected sessions by code area. Overlapping areas are not additive totals." [ chart "summary-bars" ]
                    , panel "Reports over time" "Raw observations by UTC source day; unknown source times remain separate." [ chart "summary-trend" ]
                    , details [ class "coverage-detail" ] [ summary [] [ text "Source and context coverage" ], facts counts ]
                    ]

        Sos ->
            if model.query.kind == "sos_list" then div [ class "stats-grid" ] [ stat "Reported issues" (String.fromInt (List.length doc.rows)) "Ranked by importance and latest arrival", stat "Untriaged reports" (countText "untriaged_report_count" doc.root) "Source reports awaiting a triage decision" ] else text ""

        Memory ->
            div [] [ panel "Knowledge coverage" "Raw notes and derived summaries remain attributable; expanding a summary reads its existing sources." [ facts (Wire.field "context" doc.root |> Wire.field "omitted") ], Dict.get "memory_status" model.supplemental |> Maybe.map (\health -> panel "Memory health" "Summary coverage, provider outcomes and cooldown state." [ facts (Wire.field "health" health.root) ]) |> Maybe.withDefault (text "") ]

        _ -> text ""


rowList : Model -> Document -> Html Msg
rowList model doc =
    let
        caption = if model.screen == Work && model.query.kind == "work_dashboard" then "Sessions · select for a visual breakdown" else if model.screen == Evals && model.query.kind == "eval_dashboard" then "Evaluation sets" else "Recorded observations"
        matches field keys row =
            let needle = Dict.get field model.filters |> Maybe.withDefault "" |> String.toLower in
            needle == "" || (keys |> List.map (\key -> Wire.str key row.body ++ " " ++ String.join " " (Wire.strings key row.body)) |> String.join " " |> String.toLower |> String.contains needle)
        visible =
            if model.query.kind == "eval_dashboard" then List.filter (\row -> matches "search" [ "label", "suite_identity", "target_identity", "components" ] row && matches "repository" [ "repository" ] row && matches "model" [ "models" ] row) doc.rows else if model.query.kind == "usage" then List.filter (\row -> row.kind == (if model.screen == Skills then "skill" else "tool")) doc.rows else doc.rows
    in
    panel caption (String.fromInt (List.length visible) ++ " visible · " ++ String.fromInt (List.length doc.rows) ++ " retained records")
        [ div [ class "record-list" ] (List.map (\row -> button [ class "record-row", onClick (Open row) ] [ div [ class "record-icon" ] [ text (String.left 1 (String.toUpper row.kind)) ], div [ class "record-copy" ] [ strongText row.title, small [] [ text row.subtitle ] ], span [ class "row-arrow" ] [ text "→" ] ]) visible) ]


selectedView : Model -> Row -> Html Msg
selectedView model row =
    panel (if model.screen == Work then "Selected task · " ++ row.title else row.title) row.subtitle
        [ if model.screen == Work then
            div []
                [ div [ class "selected-facts" ] [ stat "Elapsed" (exactElapsed row.body) "Exact recorded nanoseconds", stat "Kind" (Wire.str "kind" row.body) "Recorded category", stat "State / outcome" (Wire.str "state" row.body ++ " / " ++ defaultText "unknown" (Wire.str "outcome" row.body)) "Caller-reported boundaries" ]
                , facts row.body
                , button [ onClick (Navigate (Query "work_get" [ ( "subject", subject row.kind row.id ) ])) ] [ text "Open immutable task evidence →" ]
                ]
          else div [] [ facts row.body, if row.kind == "eval_record" then button [ onClick (Navigate (query "eval_get" |> with "id" row.id)) ] [ text "Open result evidence →" ] else text "" ]
        ]


exactElapsed : D.Value -> String
exactElapsed body =
    let
        ns = Wire.str "nanoseconds" (Wire.field "elapsed" body)
        padded = String.padLeft 10 '0' ns
    in
    if ns == "" then "Unknown" else String.dropRight 9 padded ++ "." ++ String.right 9 padded ++ "s"


defaultText : String -> String -> String
defaultText fallback value_ =
    if value_ == "" then fallback else value_


facts : D.Value -> Html Msg
facts root =
    case D.decodeValue (D.keyValuePairs D.value) root of
        Ok entries ->
            dl [ class "facts" ] (List.concatMap (\( key, value_ ) -> [ dt [] [ text (String.replace "_" " " key) ], dd [] [ jsonView 0 value_ ] ]) entries)

        Err _ -> jsonView 0 root


jsonView : Int -> D.Value -> Html Msg
jsonView depth value_ =
    case D.decodeValue D.string value_ of
        Ok content -> span [ class "literal" ] [ text content ]
        Err _ ->
            case D.decodeValue (D.list D.value) value_ of
                Ok items -> if List.isEmpty items then span [ class "muted" ] [ text "None recorded" ] else div [ class "json-list" ] (List.map (jsonView (depth + 1)) items)
                Err _ ->
                    case D.decodeValue (D.keyValuePairs D.value) value_ of
                        Ok entries ->
                            if depth >= 5 then details [] [ summary [] [ text "Structured metadata" ], pre [] [ text (E.encode 2 value_) ] ] else dl [ class "nested-facts" ] (List.concatMap (\( key, item ) -> [ dt [] [ text (String.replace "_" " " key) ], dd [] [ jsonView (depth + 1) item ] ]) entries)
                        Err _ -> span [ class "literal" ] [ text (if E.encode 0 value_ == "null" then "Not recorded" else E.encode 0 value_) ]


evidenceView : Model -> Document -> Html Msg
evidenceView model doc =
    let
        record = case model.query.kind of
            "concern_get" -> Wire.field "concern" doc.root
            "sos_get" -> Wire.field "detail" doc.root
            "memory_status" -> Wire.field "health" doc.root
            "work_observation" -> Wire.field "observation" doc.root
            _ -> Wire.field "record" doc.root
        body = Wire.field "body" record
        links = Wire.list "links" record
        id_ = defaultText (Wire.str "id" (Wire.field "summary" record)) (Wire.str "id" record)
        workSubject = Wire.valueAt [ "summary", "subject" ] record
        actions = case model.query.kind of
            "eval_get" -> [ button [ onClick (Navigate (query "eval_why" |> with "id" id_)) ] [ text "Why this exists · historical context" ] ]
            "work_get" ->
                [ button [ onClick (Navigate (Query "work_events" [ ( "subject", workSubject ) ])) ] [ text "Immutable observation history" ] ]
                    ++ (Wire.strings "work_session_ids" (Wire.field "summary" record) |> List.concatMap (workMembershipActions workSubject))
            "concern_get" -> [ button [ onClick (Navigate (query "concern_history" |> with "concern_id" id_)) ] [ text "Full encounter and remedy history" ] ]
            _ -> []
    in
    if E.encode 0 record == "null" then
        panel "Record unavailable" "This record was not returned for the selected project. Its identity may belong to another scope or be absent from retained history." []
    else panel (defaultText "Source evidence" (Wire.str "title" record)) "Immutable source records, motivations and retained metadata. Artifacts are referenced without fetching their bytes."
        [ div [ class "action-row" ] actions
        , if E.encode 0 body /= "null" then facts body else text ""
        , if model.query.kind == "concern_get" then div [] [ facts (Wire.field "summary" record), observationTimeline (Wire.list "history" record), facts (Wire.field "session_titles" record) ] else facts record
        , if List.isEmpty links then text "" else div [ class "evidence-links" ] [ h3 [] [ text "Related evidence" ], div [ class "action-row" ] (List.map (\link -> relatedLink model link) links) ]
        ]


workMembershipActions : D.Value -> String -> List (Html Msg)
workMembershipActions current sessionId =
    [ button [ onClick (Navigate (query "work_dashboard" |> with "work_session_id" sessionId)) ] [ text ("Session waterfall · " ++ sessionId) ]
    , button [ onClick (Navigate (Query "work_timeline" ([ ( "work_session_id", E.string sessionId ) ] ++ (if Wire.str "scope" current == "activity" then [ ( "parent_activity_id", E.string (Wire.str "id" current) ) ] else [])))) ] [ text "Browse direct children" ]
    ]


observationTimeline : List D.Value -> Html Msg
observationTimeline observations =
    div [ class "observation-timeline" ]
        (List.map
            (\observation ->
                let
                    input_ = Wire.field "input" observation
                    body = Wire.field "body" input_
                in
                div [ class "observation-step" ]
                    [ span [ class "timeline-dot" ] []
                    , header [] [ h3 [] [ text (Wire.str "event" body) ], small [] [ text (Wire.str "observed_at" input_) ] ]
                    , facts body
                    , details [] [ summary [] [ text "Source, attribution and receipt metadata" ], facts observation ]
                    ]
            )
            observations
        )


relatedLink : Model -> D.Value -> Html Msg
relatedLink model link =
    let
        target = Wire.field "target" link
        targetId = defaultText (Wire.str "id" target) (Wire.str "target_id" link)
        targetKind = Wire.str "kind" target
        wanted = if model.screen == Evals || targetKind == "eval" then Just (query "eval_get" |> with "id" targetId) else if List.member targetKind [ "session", "activity" ] then Just (Query "work_get" [ ( "subject", subject targetKind targetId ) ]) else Nothing
    in
    wanted |> Maybe.map (\q -> button [ onClick (Navigate q) ] [ text (Wire.str "relation" link ++ " → " ++ targetId) ]) |> Maybe.withDefault (div [ class "reference" ] [ text (Wire.str "relation" link), facts target ])


pagination : Model -> Document -> Html Msg
pagination model doc =
    let
        hasMore = doc.next /= Nothing || (D.decodeValue (D.field "next_offset" (D.nullable D.int)) doc.root |> Result.withDefault Nothing) /= Nothing || model.query.kind == "papercuts" && (D.decodeValue (D.field "truncated" D.bool) doc.root |> Result.withDefault False)
    in
    if hasMore then div [ class "pagination" ] [ p [] [ text "More retained records are available." ], button [ onClick More, disabled model.data.loading ] [ text "Next page →" ] ] else text ""
