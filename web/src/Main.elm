-- @feature observability-ui
-- @spec docs/features/observability-ui.md
-- @entrypoint main
-- @boundary browser-api: all domain data is decoded by Wire; credentials exist only in Elm form state until login completes.
port module Main exposing (main)

import Browser
import Browser.Navigation as Nav
import Dict exposing (Dict)
import Html exposing (Html, a, aside, button, code, div, footer, form, h1, h2, h3, header, input, label, main_, nav, option, p, pre, section, select, small, span, text)
import Html.Attributes exposing (attribute, autocomplete, class, disabled, href, id, name, selected, style, type_, value)
import Html.Events exposing (on, onClick, onInput, onSubmit)
import Http
import Icon
import Json.Decode as D
import Json.Encode as E
import State
import Time
import Url exposing (Url)
import Waterfall
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


type alias ViewerSession =
    { authenticated : Bool, username : String, defaultProjectId : Maybe String, globalProjectReads : Bool }


{-| Inline expansions load their evidence on demand and keep it while open. -}
type Detail
    = Loading
    | Failed String
    | Loaded Document


type alias Route =
    { screen : Screen, project : Maybe String, focus : Maybe String }


type alias Model =
    { key : Nav.Key
    , screen : Screen
    , authenticated : Maybe Bool
    , username : String
    , password : String
    , loginError : Maybe String
    , project : Maybe String
    , defaultProjectId : Maybe String
    , globalProjectReads : Bool
    , projects : List Row
    , projectsError : Maybe String
    , focus : Maybe String
    , data : State.Remote Document
    , polling : State.Remote Float
    , supplemental : Dict String Document
    , details : Dict String Detail
    , expanded : Dict String ()
    , query : Query
    , trail : List Query
    , selected : Maybe String
    , zoom : Float
    , mode : String
    , durationMode : String
    , navigationOpen : Bool
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
    | ChangeFocus String
    | Navigate Query
    | Back
    | Refresh
    | Tick Time.Posix
    | Received Int String Bool (Result Http.Error String)
    | DetailReceived String String (Result Http.Error String)
    | PollingReceived Int (Result Http.Error String)
    | Toggle String Query
    | SelectNode String
    | ChartSelected D.Value
    | Zoom Float
    | Mode String
    | Keyboard String
    | More
    | DurationMode String
    | ToggleNavigation


main : Program () Model Msg
main =
    Browser.application { init = init, view = view, update = update, subscriptions = subscriptions, onUrlRequest = UrlRequested, onUrlChange = UrlChanged }


init : () -> Url -> Nav.Key -> ( Model, Cmd Msg )
init _ url key =
    let
        wanted = route url
    in
    ( { key = key, screen = wanted.screen, authenticated = Nothing, username = "", password = "", loginError = Nothing, project = wanted.project, defaultProjectId = Nothing, globalProjectReads = True, projects = [], projectsError = Nothing, focus = wanted.focus, data = State.empty, polling = State.empty, supplemental = Dict.empty, details = Dict.empty, expanded = Dict.empty, query = primary wanted.screen wanted.focus, trail = [], selected = Nothing, zoom = 1, mode = "", durationMode = "average", navigationOpen = False }
    , http "GET" "session" Http.emptyBody Session
    )



-- ROUTES


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
        Coordination -> "Which sessions overlapped, and what Nexus told each agent about it."
        Work -> "Where time goes, and how each human prompt branched into work."
        Evals -> "How each agent measured over time, under which definition."
        Tools -> "How agents interface with the world — and where a better tool is missing."
        Skills -> "The guidance agents actually use, across the last seven days."
        Concerns -> "Recurring code problems, their affected areas, and the remedies already tried."
        Papercuts -> "Development friction reported by agents, most encountered first."
        Sos -> "Unusual situations encountered by agents inside running applications."
        Memory -> "Durable knowledge and the summaries built from it."


{-| The selection a screen's picker controls is part of its URL, so browser history and links restore it. -}
focusParam : Screen -> Maybe String
focusParam screen =
    case screen of
        Coordination -> Just "session"
        Work -> Just "session"
        Evals -> Just "set"
        _ -> Nothing


route : Url -> Route
route url =
    let
        screen = List.filter (\candidate -> String.endsWith ("/" ++ slug candidate) url.path) screens |> List.head |> Maybe.withDefault Coordination
        params = url.query |> Maybe.map parameters |> Maybe.withDefault Dict.empty
    in
    { screen = screen, project = Dict.get "project" params, focus = focusParam screen |> Maybe.andThen (\key -> Dict.get key params) }


parameters : String -> Dict String String
parameters raw =
    String.split "&" raw
        |> List.filterMap
            (\pair ->
                case String.split "=" pair of
                    [ key, encoded ] -> Maybe.map2 Tuple.pair (Url.percentDecode key) (Url.percentDecode (String.replace "+" " " encoded)) |> Maybe.andThen (\( k, v ) -> if v == "" then Nothing else Just ( k, v ))
                    _ -> Nothing
            )
        |> Dict.fromList


location : Screen -> Maybe String -> Maybe String -> String
location screen project focus =
    let
        pairs = [ ( Just "project", project ), ( focusParam screen, focus ) ] |> List.filterMap (\( key, item ) -> Maybe.map2 (\k v -> k ++ "=" ++ Url.percentEncode v) key item)
    in
    "/ui/" ++ slug screen ++ (if List.isEmpty pairs then "" else "?" ++ String.join "&" pairs)



-- READS


query : String -> Query
query kind =
    Query kind []


with : String -> String -> Query -> Query
with key value_ wanted =
    { wanted | fields = wanted.fields ++ [ ( key, E.string value_ ) ] }


primary : Screen -> Maybe String -> Query
primary screen focus =
    let
        focused kind key = focus |> Maybe.map (\id -> query kind |> with key id) |> Maybe.withDefault (query kind)
    in
    case screen of
        Coordination -> query "coordination"
        Work -> focused "work_dashboard" "work_session_id"
        Evals -> focused "eval_dashboard" "group_id"
        Tools -> query "analytics_summary"
        Skills -> query "usage"
        Concerns -> query "concern_dashboard"
        Papercuts -> query "papercuts"
        Sos -> query "sos_list"
        Memory -> query "memory_context"


{-| Companion reads that complete a screen: its picker index, focused evidence, or health. -}
companions : Model -> List ( String, Query )
companions model =
    case model.screen of
        Coordination ->
            ( "index", query "coordination_session_index" ) :: (model.focus |> Maybe.map (\id -> [ ( "timeline", query "coordination_session_timeline" |> with "session_id" id ) ]) |> Maybe.withDefault [])

        Evals ->
            if model.focus == Nothing then [] else [ ( "groups", query "eval_dashboard" ) ]

        Memory ->
            [ ( "health", query "memory_status" ) ]

        _ ->
            []


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
        extras = if List.isEmpty model.trail then companions next else []
    in
    if scoped model.screen && model.project == Nothing && wanted.kind /= "memory_expand" then
        ( { next | data = State.accept pending.generation (Err "Choose a project to inspect its history.") pending }, Cmd.none )

    else
        ( next, Cmd.batch (read model.project wanted (Received pending.generation wanted.kind True) :: List.map (\( key, q ) -> read model.project q (Received pending.generation key False)) extras) )


{-| A new screen, project or focus starts from that screen's primary read with no stale selection. -}
reset : Route -> Model -> ( Model, Cmd Msg )
reset wanted model =
    let
        sameScreen = wanted.screen == model.screen && wanted.project == model.project
    in
    load (primary wanted.screen wanted.focus)
        { model
            | screen = wanted.screen
            , project = wanted.project
            , focus = wanted.focus
            , data = { value = Nothing, generation = model.data.generation, loading = False, error = Nothing }
            , supplemental = if sameScreen then Dict.remove "timeline" model.supplemental else Dict.empty
            , details = if sameScreen then model.details else Dict.empty
            , expanded = if sameScreen then model.expanded else Dict.empty
            , trail = []
            , selected = Nothing
            , zoom = 1
            , mode = if sameScreen then model.mode else ""
            , navigationOpen = False
        }



-- UPDATE


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        ToggleNavigation ->
            ( { model | navigationOpen = not model.navigationOpen }, Cmd.none )

        UrlRequested (Browser.Internal url) ->
            ( model, Nav.pushUrl model.key (Url.toString url) )

        UrlRequested (Browser.External url) ->
            ( model, Nav.load url )

        UrlChanged url ->
            let
                wanted = route url
                -- Every in-app link carries the project, so an absent one means all projects where permitted.
                project = if model.globalProjectReads then wanted.project else wanted.project |> orElse model.project
            in
            if model.authenticated /= Just True then
                ( { model | screen = wanted.screen, project = project, focus = wanted.focus }, Cmd.none )

            else
                reset { wanted | project = project } model

        Username username ->
            ( { model | username = username }, Cmd.none )

        Password password ->
            ( { model | password = password }, Cmd.none )

        Login ->
            ( { model | loginError = Nothing }, request "login" (E.object [ ( "username", E.string model.username ), ( "password", E.string model.password ) ]) LoggedIn )

        Session result ->
            case result |> Result.mapError httpError |> Result.andThen sessionDecoder of
                Ok session ->
                    if session.authenticated then authenticate session model else ( { model | authenticated = Just False }, Cmd.none )

                Err reason ->
                    ( { model | authenticated = Just False, loginError = Just reason }, Cmd.none )

        LoggedIn result ->
            case result |> Result.mapError httpError |> Result.andThen sessionDecoder of
                Ok session ->
                    if session.authenticated then
                        authenticate session model
                    else
                        ( { model | authenticated = Just False, password = "", loginError = Just "Sign-in failed. Check your configured credentials." }, Cmd.none )

                Err reason ->
                    ( { model | authenticated = Just False, password = "", loginError = Just reason }, Cmd.none )

        Logout ->
            ( model, request "logout" (E.object []) LoggedOut )

        LoggedOut result ->
            case result of
                Ok _ ->
                    ( signedOut model, Cmd.none )

                Err error ->
                    ( { model | loginError = Just (httpError error) }, Cmd.none )

        Projects result ->
            case result |> Result.mapError httpError |> Result.andThen (Wire.decode "projects") of
                Ok doc ->
                    reset { screen = model.screen, project = model.project |> orElse (List.head doc.rows |> Maybe.map .id), focus = model.focus } { model | projectsError = Nothing, projects = doc.rows }

                Err reason ->
                    ( { model | projectsError = Just reason, data = { value = Nothing, generation = model.data.generation + 1, error = Just ("The project list could not be loaded. " ++ reason), loading = False } }, Cmd.none )

        ChangeProject project ->
            if project == "" && not model.globalProjectReads then
                ( model, Cmd.none )
            else
                ( model, Nav.pushUrl model.key (location model.screen (if project == "" then Nothing else Just project) Nothing) )

        ChangeFocus focus ->
            ( model, Nav.pushUrl model.key (location model.screen model.project (if focus == "" then Nothing else Just focus)) )

        Navigate wanted ->
            load wanted { model | trail = model.query :: model.trail, selected = Nothing }

        Back ->
            case model.trail of
                previous :: rest ->
                    load previous { model | trail = rest, selected = Nothing }

                [] ->
                    reset { screen = model.screen, project = model.project, focus = model.focus } model

        Refresh ->
            let
                ( loaded, reading ) = load model.query model
                ( refreshed, configuration ) = if model.polling.error /= Nothing && not model.polling.loading then bootstrapPolling loaded else ( loaded, Cmd.none )
            in
            ( refreshed, Cmd.batch [ reading, configuration ] )

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
                        let expired = signedOut model in
                        ( { expired | loginError = Just "Your session expired. Sign in to continue." }, Cmd.none )

                    _ ->
                        let
                            wireKind = if isPrimary then kind else companionKind kind
                            decoded = result |> Result.mapError httpError |> Result.andThen (Wire.decode wireKind)
                            updated = if isPrimary then { model | data = State.accept generation decoded model.data } else { model | supplemental = decoded |> Result.map (\doc -> Dict.insert kind doc model.supplemental) |> Result.withDefault model.supplemental }
                        in
                        ( updated, draw updated )

        DetailReceived key kind result ->
            case result of
                Err (Http.BadStatus 401) ->
                    let expired = signedOut model in
                    ( { expired | loginError = Just "Your session expired. Sign in to continue." }, Cmd.none )

                _ ->
                    let
                        detail = case result |> Result.mapError httpError |> Result.andThen (Wire.decode kind) of
                            Ok doc -> Loaded doc
                            Err reason -> Failed reason
                        next = { model | details = Dict.insert key detail model.details }
                    in
                    ( next, draw next )

        PollingReceived generation result ->
            if model.authenticated /= Just True || generation /= model.polling.generation then
                ( model, Cmd.none )
            else
                case result of
                    Err (Http.BadStatus 401) ->
                        let expired = signedOut model in
                        ( { expired | loginError = Just "Your session expired. Sign in to continue." }, Cmd.none )

                    _ ->
                        let decoded = result |> Result.mapError httpError |> Result.andThen (Wire.decode "coordination") |> Result.andThen (.root >> Wire.refreshInterval) in
                        ( { model | polling = State.accept generation decoded model.polling }, Cmd.none )

        Toggle key wanted ->
            if Dict.member key model.expanded then
                ( { model | expanded = Dict.remove key model.expanded }, Cmd.none )

            else
                let
                    opened = { model | expanded = Dict.insert key () model.expanded }
                    known = case Dict.get key model.details of
                        Just (Loaded _) -> True
                        _ -> False
                in
                if wanted.kind == "" || known then
                    ( opened, draw opened )
                else
                    ( { opened | details = Dict.insert key Loading model.details }, read model.project wanted (DetailReceived key wanted.kind) )

        SelectNode nodeId ->
            let
                next = { model | selected = Just nodeId }
            in
            ( next, draw next )

        ChartSelected event ->
            case D.decodeValue (D.map2 Tuple.pair (D.field "chart" D.string) (D.field "id" D.string)) event of
                Ok ( "work-waterfall", nodeId ) ->
                    update (SelectNode nodeId) model

                Ok ( "summary-bars", areaId ) ->
                    if model.screen == Concerns then
                        update (Navigate (Query "concern_list" [ ( "filter", E.object [ ( "area_id", E.string areaId ) ] ) ])) model
                    else
                        ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        Zoom factor ->
            let
                next = { model | zoom = clamp 0.5 24 (model.zoom * factor) }
            in
            ( next, draw next )

        Mode mode ->
            load (modeQuery model.screen mode) { model | mode = mode, trail = [], selected = Nothing }

        Keyboard key ->
            keyboard key model

        More ->
            more model

        DurationMode mode ->
            let next = { model | durationMode = mode } in
            ( next, draw next )


companionKind : String -> String
companionKind key =
    case key of
        "index" -> "coordination_session_index"
        "timeline" -> "coordination_session_timeline"
        "groups" -> "eval_dashboard"
        "health" -> "memory_status"
        _ -> key


sessionDecoder : String -> Result String ViewerSession
sessionDecoder raw =
    D.decodeString
        (D.map4 ViewerSession
            (D.field "authenticated" D.bool)
            (D.oneOf [ D.field "username" D.string, D.succeed "" ])
            (Wire.optional "default_project_id" (D.nullable D.string) Nothing)
            (Wire.optional "global_project_reads" D.bool True)
        ) raw
        |> Result.mapError D.errorToString


authenticate : ViewerSession -> Model -> ( Model, Cmd Msg )
authenticate session model =
    let
        ( configured, configuration ) = bootstrapPolling { model | authenticated = Just True, username = session.username, password = "", loginError = Nothing, project = model.project |> orElse session.defaultProjectId, defaultProjectId = session.defaultProjectId, globalProjectReads = session.globalProjectReads }
    in
    ( configured, Cmd.batch [ read Nothing (query "projects") Projects, configuration ] )


bootstrapPolling : Model -> ( Model, Cmd Msg )
bootstrapPolling model =
    let pending = State.begin model.polling in
    ( { model | polling = pending }, read (if model.globalProjectReads then Nothing else model.defaultProjectId) (query "coordination") (PollingReceived pending.generation) )


signedOut : Model -> Model
signedOut model =
    { model | authenticated = Just False, password = "", data = State.empty, supplemental = Dict.empty, details = Dict.empty, expanded = Dict.empty, selected = Nothing, polling = { value = Nothing, generation = model.polling.generation + 1, loading = False, error = Nothing } }


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


modeQuery : Screen -> String -> Query
modeQuery screen mode =
    query <|
        case screen of
            Tools -> if mode == "capabilities" then "analytics_catalog" else if mode == "opportunities" then "analytics_opportunities" else "analytics_summary"
            Concerns -> if mode == "all" then "concern_list" else "concern_dashboard"
            _ -> (primary screen Nothing).kind


more : Model -> ( Model, Cmd Msg )
more model =
    case model.data.value of
        Nothing -> ( model, Cmd.none )
        Just doc ->
            let
                next = case model.query.kind of
                    "papercuts" -> if D.decodeValue (D.field "truncated" D.bool) doc.root == Ok True then List.reverse doc.rows |> List.head |> Maybe.map (\row -> E.object [ ( "upvote_count", E.int (round (Wire.number "upvote_count" row.body)) ), ( "updated_at", E.string (Wire.str "updated_at" row.body) ), ( "id", E.string row.id ) ]) else Nothing
                    "analytics_catalog" -> D.decodeValue (D.field "next_offset" (D.nullable D.int)) doc.root |> Result.withDefault Nothing |> Maybe.map E.int
                    "analytics_examples" -> D.decodeValue (D.field "next_offset" (D.nullable D.int)) doc.root |> Result.withDefault Nothing |> Maybe.map E.int
                    _ -> doc.next
                wanted cursor =
                    if List.member model.query.kind [ "concern_list", "concern_dashboard" ] then
                        let
                            active = List.filter (\( key, _ ) -> key == "filter") model.query.fields |> List.head |> Maybe.map Tuple.second |> Maybe.andThen (D.decodeValue (D.keyValuePairs D.value) >> Result.toMaybe) |> Maybe.withDefault []
                        in
                        Query model.query.kind [ ( "filter", E.object (( "after", cursor ) :: List.filter (\( key, _ ) -> key /= "after") active) ) ]
                    else
                        { kind = model.query.kind, fields = List.filter (\( k, _ ) -> k /= "after" && k /= "offset") model.query.fields ++ [ ( if String.startsWith "analytics_" model.query.kind then "offset" else "after", cursor ) ] }
            in
            next |> Maybe.map (\cursor -> load (wanted cursor) { model | trail = model.query :: model.trail, selected = Nothing }) |> Maybe.withDefault ( model, Cmd.none )



-- WATERFALL


waterfallNodes : Document -> List Waterfall.Node
waterfallNodes doc =
    let
        root = Wire.field "selected_session" doc.root
        rootId = Wire.str "id" (Wire.field "subject" root)
        rootNode = { id = rootId, parent = Nothing, label = Wire.str "title" root, kind = "work.session", start = maybeText (Wire.field "started_at" root), end = maybeText (Wire.field "finished_at" root), queued = maybeText (Wire.field "queued_at" root), count = 1, body = root }
    in
    if rootId == "" then [] else Waterfall.build rootNode (List.map (\node -> { id = node.id, parent = node.parent, label = node.label, kind = node.kind, start = node.start, end = node.end, queued = node.queued, count = 1, body = node.body }) doc.intervals)


keyboard : String -> Model -> ( Model, Cmd Msg )
keyboard key model =
    if model.screen /= Work || model.focus == Nothing then
        ( model, Cmd.none )
    else
        let
            nodes = model.data.value |> Maybe.map waterfallNodes |> Maybe.withDefault []
            rootId = List.head nodes |> Maybe.map .id |> Maybe.withDefault ""
            selectedId = model.selected |> Maybe.withDefault rootId
            index = nodes |> List.indexedMap Tuple.pair |> List.filter (\( _, node ) -> node.id == selectedId) |> List.head |> Maybe.map Tuple.first |> Maybe.withDefault 0
            parentOf node = node.parent |> Maybe.withDefault rootId
            current = List.drop index nodes |> List.head
            picked = case key of
                "ArrowDown" -> List.drop (index + 1) nodes |> List.head
                "ArrowUp" -> List.drop (max 0 (index - 1)) nodes |> List.head
                "ArrowLeft" -> current |> Maybe.andThen (\node -> List.filter (\candidate -> candidate.id == parentOf node && node.id /= rootId) nodes |> List.head)
                "ArrowRight" -> List.filter (\node -> node.id /= rootId && parentOf node == selectedId) nodes |> List.head
                _ -> Nothing
        in
        picked |> Maybe.map (\node -> update (SelectNode node.id) model) |> Maybe.withDefault ( model, Cmd.none )


subscriptions : Model -> Sub Msg
subscriptions model =
    Sub.batch [ chartSelection ChartSelected, if model.authenticated == Just True then model.polling.value |> Maybe.map (\interval -> Time.every interval Tick) |> Maybe.withDefault Sub.none else Sub.none ]



-- CHARTS


draw : Model -> Cmd Msg
draw model =
    case model.data.value of
        Nothing -> Cmd.none
        Just doc -> charts (E.list identity (screenCharts model doc))


screenCharts : Model -> Document -> List E.Value
screenCharts model doc =
    case model.screen of
        Work ->
            if model.focus /= Nothing then
                waterfallChart model doc
            else
                [ barsChart "summary-bars" (workBars model doc) ]

        Evals ->
            if model.focus == Nothing then [] else List.indexedMap (\index metric -> encodeTrend ("metric-" ++ String.fromInt index) False metric.points) doc.metrics

        Concerns ->
            if model.query.kind == "concern_dashboard" then [ barsChart "summary-bars" doc.bars, encodeTrend "summary-trend" True doc.points ] else []

        Tools ->
            if model.query.kind == "analytics_summary" then commandCharts model doc else []

        _ ->
            []


workBars : Model -> Document -> List Wire.Bar
workBars model doc =
    let
        metricKey = if model.durationMode == "total" then "total_seconds" else "average_seconds"
    in
    Wire.list "activity_kinds" doc.root
        |> List.filter (\row -> Wire.str "accounting_role" row /= "container")
        |> List.map
            (\row ->
                let
                    known = Wire.number "known_count" row > 0
                in
                Wire.Bar (Wire.str "kind" row) (Wire.str "kind" row) (if known then Just (Wire.number metricKey row) else Nothing) ((if known then duration (Wire.number metricKey row) else "Unknown") ++ " · " ++ countText "known_count" row ++ "/" ++ countText "count" row ++ " timed")
            )


barsChart : String -> List Wire.Bar -> E.Value
barsChart chartId bars =
    E.object [ ( "id", E.string chartId ), ( "type", E.string "bars" ), ( "bars", E.list (\bar -> E.object [ ( "id", E.string bar.id ), ( "label", E.string bar.label ), ( "value", bar.value |> Maybe.map E.float |> Maybe.withDefault E.null ), ( "detail", E.string bar.detail ) ]) bars ) ]


waterfallChart : Model -> Document -> List E.Value
waterfallChart model doc =
    case waterfallNodes doc of
        root :: rest ->
            [ E.object [ ( "id", E.string "work-waterfall" ), ( "type", E.string "waterfall" ), ( "selected", model.selected |> Maybe.map E.string |> Maybe.withDefault E.null ), ( "zoom", E.float model.zoom ), ( "spans", E.list (encodeNode root.id) (root :: rest) ) ] ]

        [] ->
            []


{-| Tool categories expanded in place draw one time histogram per command. -}
commandCharts : Model -> Document -> List E.Value
commandCharts model doc =
    doc.rows
        |> List.indexedMap Tuple.pair
        |> List.concatMap
            (\( index, row ) ->
                case ( Dict.member (commandsKey row.id) model.expanded, Dict.get (commandsKey row.id) model.details ) of
                    ( True, Just (Loaded commands) ) ->
                        Wire.list "commands" commands.root
                            |> List.indexedMap (\position command -> E.object [ ( "id", E.string (commandChartId index position) ), ( "type", E.string "histogram" ), ( "binSeconds", E.float (Wire.number "bin_seconds" commands.root) ), ( "bins", E.list (\bin -> E.object [ ( "start", E.string (Wire.str "start" bin) ), ( "count", E.float (Wire.number "count" bin) ) ]) (Wire.list "bins" command) ) ])

                    _ ->
                        []
            )


commandsKey : String -> String
commandsKey categoryId =
    "commands:" ++ categoryId


commandChartId : Int -> Int -> String
commandChartId category command =
    "commands-" ++ String.fromInt category ++ "-" ++ String.fromInt command


maybeText : D.Value -> Maybe String
maybeText value_ =
    D.decodeValue (D.nullable D.string) value_ |> Result.withDefault Nothing


encodeNode : String -> Waterfall.Node -> E.Value
encodeNode rootId node =
    E.object [ ( "id", E.string node.id ), ( "label", E.string node.label ), ( "kind", E.string node.kind ), ( "parent", (if node.id == rootId then Nothing else node.parent |> orElse (Just rootId)) |> Maybe.map E.string |> Maybe.withDefault E.null ), ( "start", encodeTime node.start ), ( "end", encodeTime node.end ), ( "queued", encodeTime node.queued ) ]


encodeTime : Maybe String -> E.Value
encodeTime value_ =
    -- JS Date parsing is confined to chart positioning. Source timestamps remain literal in Elm.
    value_ |> Maybe.map E.string |> Maybe.withDefault E.null


encodeTrend : String -> Bool -> List Wire.Point -> E.Value
encodeTrend chartId connected points =
    E.object [ ( "id", E.string chartId ), ( "type", E.string "trend" ), ( "line", E.bool connected ), ( "points", E.list (\pt -> E.object [ ( "id", E.string pt.id ), ( "label", E.string pt.label ), ( "at", E.string pt.at ), ( "value", pt.value |> Maybe.map E.float |> Maybe.withDefault E.null ) ]) points ) ]



-- VIEW


view : Model -> Browser.Document Msg
view model =
    { title = "Nexus · " ++ title model.screen
    , body =
        [ case model.authenticated of
            Nothing -> div [ class "connection" ] [ div [ class "brand-mark" ] [ Icon.view Icon.Layers ], h2 [] [ text "Connecting to Nexus" ], p [] [ text "Opening your observation workspace…" ] ]
            Just False -> loginView model
            Just True -> workspace model
        ]
    }


loginView : Model -> Html Msg
loginView model =
    main_ [ class "login-shell" ]
        [ div [ class "login-side" ]
            [ div [ class "login-brand wordmark" ] [ Icon.view Icon.Layers, text "nexus", small [] [ text "Observability" ] ]
            , form [ class "login-card", onSubmit Login ]
                [ h1 [] [ text "Welcome to Nexus" ], p [ class "muted" ] [ text "Sign in to inspect your agent workspace." ]
                , label [] [ text "Username", input [ name "username", autocomplete True, attribute "autocomplete" "username", value model.username, onInput Username, attribute "required" "true" ] [] ]
                , label [] [ text "Password", input [ name "password", type_ "password", attribute "autocomplete" "current-password", value model.password, onInput Password, attribute "required" "true" ] [] ]
                , model.loginError |> Maybe.map (\reason -> div [ class "notice error", attribute "role" "alert" ] [ text reason ]) |> Maybe.withDefault (text "")
                , button [ class "primary", type_ "submit" ] [ text "Sign in", Icon.view Icon.Forward ]
                , div [ class "login-note" ] [ Icon.view Icon.Lock, text "Read-only access. Credentials are configured at setup." ]
                ]
            , p [ class "login-footer" ] [ text "Sessions, evaluations, and the evidence behind them." ]
            ]
        ]


workspace : Model -> Html Msg
workspace model =
    div [ class (if model.navigationOpen then "workspace navigation-open" else "workspace") ]
        [ aside [ class "sidebar", id "workspace-navigation" ]
            [ a [ class "wordmark", href (location Coordination model.project Nothing) ] [ Icon.view Icon.Layers, text "nexus", small [] [ text "Observe" ] ]
            , div [ class "nav-caption" ] [ text "Workspace" ]
            , nav [ attribute "aria-label" "Workspace" ] (List.map (\screen -> a [ href (location screen model.project Nothing), class (if model.screen == screen then "nav-item active" else "nav-item"), attribute "aria-current" (if model.screen == screen then "page" else "false") ] [ Icon.view (screenIcon screen), span [] [ text (title screen) ] ]) screens)
            , div [ class "sidebar-bottom" ] [ div [ class "read-only" ] [ Icon.view Icon.Lock, text "Read-only workspace" ], p [] [ text "Durable agent evidence" ] ]
            ]
        , div [ class "main-shell" ]
            [ header [ class "topbar" ]
                [ button [ class "navigation-toggle quiet", onClick ToggleNavigation, attribute "aria-label" "Toggle workspace navigation", attribute "aria-controls" "workspace-navigation", attribute "aria-expanded" (if model.navigationOpen then "true" else "false") ] [ Icon.view (if model.navigationOpen then Icon.Close else Icon.Menu), text "Menu" ]
                , div [ class "breadcrumb" ] [ text "Workspace", span [] [ text "/" ], text (title model.screen) ]
                , div [ class "top-actions" ] [ span [ class "account" ] [ text model.username ], button [ onClick Logout, class "quiet" ] [ Icon.view Icon.Logout, text "Sign out" ] ]
                ]
            , main_ [ class "content" ]
                [ div [ class "page-heading" ] [ div [] [ h1 [] [ text (title model.screen) ], p [] [ text (intro model.screen) ] ] ]
                , toolbar model
                , statusView model
                , case model.data.value of
                    Nothing -> div [ class "empty-state" ] [ div [ class "empty-glyph" ] [ Icon.view Icon.Inbox ], h2 [] [ text (if model.data.loading then "Loading observations" else "Ready when you are") ], p [] [ text (if model.data.loading then "Retrieving the latest retained evidence…" else "Select a project or refresh to retrieve evidence.") ] ]
                    Just doc -> documentView model doc
                ]
            , footer [ class "page-footer" ] [ text "Nexus · Durable agent evidence", span [] [ text "Read-only" ] ]
            ]
        ]


screenIcon : Screen -> Icon.Icon
screenIcon screen =
    case screen of
        Coordination -> Icon.Activity
        Work -> Icon.Branch
        Evals -> Icon.Flask
        Tools -> Icon.Tools
        Skills -> Icon.Book
        Concerns -> Icon.Bug
        Papercuts -> Icon.Warning
        Sos -> Icon.Siren
        Memory -> Icon.Memory


toolbar : Model -> Html Msg
toolbar model =
    let
        current = Maybe.withDefault "" model.project
        catalog = List.map (\row -> choice current row.id (projectCaption row)) model.projects
        custom = if current /= "" && not (List.any (\row -> row.id == current) model.projects) then [ choice current current current ] else []
    in
    div [ class "toolbar" ]
        [ div [ class "scope-controls" ]
            (label [ class "scope-control" ] [ span [ class "muted" ] [ text "Project" ], select [ onInput ChangeProject, attribute "aria-label" "Project scope" ] ((if model.globalProjectReads then [ choice current "" "All projects" ] else []) ++ custom ++ catalog) ]
                :: focusPicker model
            )
        , div [ class "toolbar-right" ] [ if List.isEmpty model.trail then text "" else button [ onClick Back ] [ Icon.view Icon.Back, text "Back" ], button [ onClick Refresh, disabled model.data.loading ] [ Icon.view Icon.Refresh, text (if model.data.loading then "Refreshing…" else "Refresh") ] ]
        ]


{-| The second picker chooses what a screen focuses on: a session, or an evaluated agent. -}
focusPicker : Model -> List (Html Msg)
focusPicker model =
    let
        picker caption placeholder rows =
            let
                current = Maybe.withDefault "" model.focus
                missing = if current /= "" && not (List.any (\row -> row.id == current) rows) then [ choice current current current ] else []
            in
            [ label [ class "scope-control focus-control" ] [ span [ class "muted" ] [ text caption ], select [ onInput ChangeFocus, attribute "aria-label" caption ] (choice current "" placeholder :: missing ++ List.map (\row -> choice current row.id (focusCaption row)) rows) ] ]

        documentRows = model.data.value |> Maybe.map .rows |> Maybe.withDefault []
    in
    case model.screen of
        Coordination -> picker "Session" "Choose a session…" (Dict.get "index" model.supplemental |> Maybe.map .rows |> Maybe.withDefault [])
        Work -> picker "Session" "Choose a session…" documentRows
        Evals -> picker "Agent" "Choose an agent…" (if model.focus == Nothing then documentRows else Dict.get "groups" model.supplemental |> Maybe.map .rows |> Maybe.withDefault documentRows)
        _ -> []


focusCaption : Row -> String
focusCaption row =
    case row.kind of
        "coordination_session" -> row.title ++ " · " ++ row.subtitle
        "eval_group" -> evalCaption row
        _ ->
            let
                started = Wire.str "started_at" row.body
                active = D.decodeValue (D.field "active_seconds" D.float) row.body |> Result.toMaybe
            in
            row.title ++ (if started == "" then "" else " · " ++ timeText started) ++ (active |> Maybe.map (\seconds -> " · " ++ duration seconds ++ " active") |> Maybe.withDefault "")


evalCaption : Row -> String
evalCaption row =
    let
        agent = Wire.str "agent" row.body
    in
    (if agent /= "" && not (String.startsWith agent row.title) then agent ++ " · " else "") ++ row.title ++ (if row.subtitle == "" then "" else " · " ++ row.subtitle)


projectCaption : Row -> String
projectCaption row =
    if row.title /= row.id then
        row.title
    else if row.subtitle /= "" then
        row.subtitle
    else
        row.id


choice : String -> String -> String -> Html Msg
choice current key caption =
    option [ attribute "value" key, selected (current == key) ] [ text caption ]


statusView : Model -> Html Msg
statusView model =
    div [ attribute "aria-live" "polite" ]
        [ model.data.error |> Maybe.map (\reason -> div [ class "notice error" ] [ text reason, if model.data.value /= Nothing then span [] [ text " · Showing the last successful snapshot." ] else text "" ]) |> Maybe.withDefault (text "")
        , model.loginError |> Maybe.map (\reason -> div [ class "notice error" ] [ text reason ]) |> Maybe.withDefault (text "")
        , model.polling.error |> Maybe.map (\reason -> div [ class "notice" ] [ text ("Automatic refresh configuration is unavailable. " ++ reason ++ " Use Refresh to retry.") ]) |> Maybe.withDefault (text "")
        ]


documentView : Model -> Document -> Html Msg
documentView model doc =
    div [ class "document" ]
        [ subnav model
        , case model.screen of
            Coordination -> coordinationView model doc
            Work -> workView model doc
            Evals -> evalView model doc
            Tools -> toolsView model doc
            Skills -> skillsView model doc
            Concerns -> concernsView model doc
            Papercuts -> papercutsView model doc
            Sos -> sosView model doc
            Memory -> memoryView model doc
        , pagination model doc
        ]


subnav : Model -> Html Msg
subnav model =
    let
        modes = case model.screen of
            Tools -> [ ( "", "Activities" ), ( "capabilities", "Capabilities" ), ( "opportunities", "Opportunities" ) ]
            Concerns -> [ ( "", "Problem areas" ), ( "all", "All concerns" ) ]
            _ -> []
    in
    if List.isEmpty modes then text "" else div [ class "subnav" ] (List.map (\( mode, caption ) -> button [ class (if mode == model.mode then "active" else ""), onClick (Mode mode) ] [ text caption ]) modes)



-- SHARED PIECES


stat : String -> String -> String -> Html Msg
stat caption value_ note =
    div [ class "stat" ] [ div [ class "stat-label" ] [ text caption ], div [ class "stat-value" ] [ text value_ ], small [] [ text note ] ]


countText : String -> D.Value -> String
countText key root =
    formatCount (Wire.number key root)


{-| A count the response does not carry stays unavailable instead of becoming zero. -}
optionalCount : String -> D.Value -> String
optionalCount key root =
    D.decodeValue (D.field key D.float) root |> Result.map formatCount |> Result.withDefault "—"


formatCount : Float -> String
formatCount number =
    let
        digits = String.fromInt (round number)
        grouped = String.foldr (\char ( count, acc ) -> ( count + 1, if count > 0 && modBy 3 count == 0 && char /= '-' then String.cons char ("," ++ acc) else String.cons char acc )) ( 0, "" ) digits |> Tuple.second
    in
    if toFloat (round number) == number then grouped else String.fromFloat number


duration : Float -> String
duration seconds =
    if seconds >= 3600 then String.fromFloat (toFloat (round (seconds / 36)) / 100) ++ "h" else if seconds >= 60 then String.fromFloat (toFloat (round (seconds / 6)) / 10) ++ "m" else String.fromFloat (toFloat (round (seconds * 100)) / 100) ++ "s"


{-| Source timestamps are shown as their UTC date and minute; the literal value stays in the title. -}
timeText : String -> String
timeText stamp =
    let
        months = [ "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec" ]
        month = String.slice 5 7 stamp |> String.toInt |> Maybe.andThen (\m -> List.drop (m - 1) months |> List.head)
        day = String.slice 8 10 stamp |> String.toInt
        clock = String.slice 11 16 stamp
    in
    case ( month, day ) of
        ( Just m, Just d ) -> m ++ " " ++ String.fromInt d ++ (if String.length clock == 5 then ", " ++ clock ++ " UTC" else "")
        _ -> stamp


timeView : String -> Html Msg
timeView stamp =
    if stamp == "" then text "" else Html.time [ attribute "datetime" stamp, attribute "title" stamp ] [ text (timeText stamp) ]


panel : String -> String -> List (Html Msg) -> Html Msg
panel heading caption body =
    section [ class "panel" ] (div [ class "panel-heading" ] [ div [] [ h2 [] [ text heading ], if caption == "" then text "" else p [] [ text caption ] ] ] :: body)


guidance : String -> String -> Html Msg
guidance heading caption =
    div [ class "empty-state" ] [ div [ class "empty-glyph" ] [ Icon.view Icon.Inbox ], h2 [] [ text heading ], p [] [ text caption ] ]


chart : String -> Html Msg
chart chartId =
    div [ id chartId, class "chart" ] []


strongText : String -> Html Msg
strongText value_ =
    Html.strong [] [ text value_ ]


nothingRecorded : String -> Html Msg
nothingRecorded caption =
    p [ class "panel-empty" ] [ text caption ]


{-| Prose fields of a record, in a stable reading order. Identifiers, hashes and receipt metadata are omitted. -}
prose : D.Value -> List ( String, String )
prose record =
    [ "summary", "description", "note", "message", "details", "detail", "body", "content", "text", "reason", "remedy", "motivation", "resolution", "outcome_summary" ]
        |> List.filterMap (\key -> let content = Wire.str key record |> String.trim in if content == "" then Nothing else Just ( key, content ))


proseView : D.Value -> Html Msg
proseView record =
    case prose record of
        [] -> text ""
        fields -> div [ class "prose" ] (List.map (\( _, content ) -> p [] [ text content ]) fields)


type alias Expandable =
    { key : String, load : Query, icon : Icon.Icon, title : String, subtitle : String, aside : Html Msg }


{-| A list row that opens its evidence directly beneath itself. -}
expandable : Model -> Expandable -> (Maybe Document -> Html Msg) -> Html Msg
expandable model row render =
    let
        open = Dict.member row.key model.expanded
    in
    div [ class (if open then "record expanded" else "record") ]
        [ button [ class "record-row", onClick (Toggle row.key row.load), attribute "aria-expanded" (if open then "true" else "false") ]
            [ Icon.view row.icon, div [ class "record-copy" ] [ strongText row.title, if row.subtitle == "" then text "" else small [] [ text row.subtitle ] ], row.aside, span [ class "row-arrow" ] [ Icon.view Icon.Chevron ] ]
        , if open then div [ class "record-detail" ] [ detailView model row render ] else text ""
        ]


detailView : Model -> Expandable -> (Maybe Document -> Html Msg) -> Html Msg
detailView model row render =
    if row.load.kind == "" then
        render Nothing
    else
        case Dict.get row.key model.details of
            Just (Loaded doc) -> render (Just doc)
            Just (Failed reason) -> div [ class "notice error" ] [ text reason ]
            _ -> p [ class "panel-empty" ] [ text "Loading…" ]


plainRow : Icon.Icon -> String -> String -> Html Msg
plainRow icon heading caption =
    div [ class "record-row static" ] [ Icon.view icon, div [ class "record-copy" ] [ strongText heading, if caption == "" then text "" else small [] [ text caption ] ] ]


{-| Horizontal usage bars as accessible rows, each expanding in place. -}
barList : Model -> List ( Expandable, Float ) -> (Int -> Maybe Document -> Html Msg) -> Html Msg
barList model rows render =
    let
        maximum = rows |> List.map Tuple.second |> List.maximum |> Maybe.withDefault 1 |> max 1
    in
    if List.isEmpty rows then
        nothingRecorded "No recorded observations in this scope."
    else
        div [ class "bar-list" ]
            (List.indexedMap
                (\index ( row, amount ) ->
                    let
                        open = Dict.member row.key model.expanded
                    in
                    div [ class (if open then "bar-item expanded" else "bar-item") ]
                        [ button [ class "bar-row", onClick (Toggle row.key row.load), attribute "aria-expanded" (if open then "true" else "false") ]
                            [ span [ class "bar-label" ] [ text row.title ], span [ class "bar-value" ] [ text row.subtitle ], span [ class "bar-track", attribute "aria-hidden" "true" ] [ span [ class "bar-fill", style "width" (String.fromFloat (max 0.4 (amount / maximum * 100)) ++ "%") ] [] ] ]
                        , if open then div [ class "bar-detail" ] [ detailView model row (render index) ] else text ""
                        ]
                )
                rows
            )



-- COORDINATION


coordinationView : Model -> Document -> Html Msg
coordinationView model doc =
    let
        counts = Wire.field "counts" doc.root
    in
    div []
        [ div [ class "stats-grid" ]
            [ stat "Recent sessions" (countText "active_sessions" counts) "Seen within the activity window"
            , stat "Live claims" (countText "active_claims" counts) "Paths agents currently hold"
            , stat "Conflicts · 7 days" (optionalCount "recent_conflicts" counts) "Overlaps detected between sessions"
            , stat "Messages · 7 days" (optionalCount "recent_advisories" counts) "Warnings Nexus delivered to agents"
            ]
        , case ( model.focus, Dict.get "timeline" model.supplemental ) of
            ( Nothing, _ ) -> guidance "Choose a session" "Pick a session above to see the overlaps it ran into and the messages Nexus sent about them."
            ( Just _, Nothing ) -> guidance "Loading the session" "Retrieving its conflicts and messages…"
            ( Just _, Just timeline ) -> conversation timeline
        ]


{-| A session's coordination reads as a conversation: what overlapped, and who Nexus told. -}
conversation : Document -> Html Msg
conversation timeline =
    let
        session = Wire.field "session" timeline.root
        entries = Wire.list "entries" timeline.root
        worktree = Wire.str "worktree" session |> String.split "/" |> List.reverse |> List.head |> Maybe.withDefault ""
        caption = [ Wire.str "agent" session, worktree, if Wire.str "last_seen_at" session == "" then "" else "last seen " ++ timeText (Wire.str "last_seen_at" session) ] |> List.filter ((/=) "") |> String.join " · "
        heading = if E.encode 0 session == "null" then "Session" else Wire.sessionName session
    in
    panel heading caption
        [ if List.isEmpty entries then
            nothingRecorded "Nexus recorded no overlaps or messages for this session."
          else
            div [ class "conversation" ] (List.map conversationEntry entries)
        , if D.decodeValue (D.field "truncated" D.bool) timeline.root == Ok True then p [ class "chart-note" ] [ text "Showing the most recent entries." ] else text ""
        ]


conversationEntry : D.Value -> Html Msg
conversationEntry entry =
    let
        counterpart = Wire.field "counterpart" entry
        other = if E.encode 0 counterpart == "null" then "another session" else Wire.sessionName counterpart
        message = Wire.str "message" entry
        sent = Wire.str "direction" entry == "sent"
        ( speaker, headline, tone ) =
            case Wire.str "type" entry of
                "conflict_detected" -> ( "Nexus", "Overlap detected with " ++ other, "conflict" )
                "conflict_resolved" -> ( "Nexus", "Overlap with " ++ other ++ " ended", "resolved" )
                _ -> ( if sent then "Nexus → " ++ other else "Nexus → this session", "", if sent then "advisory sent" else "advisory" )
    in
    div [ class ("message " ++ tone) ]
        [ header [] [ span [ class "message-from" ] [ text speaker ], small [] [ timeView (Wire.str "at" entry) ] ]
        , if headline == "" then text "" else p [ class "message-headline" ] [ text headline ]
        , if message == "" || headline /= "" && message == headline then text "" else p [] [ text message ]
        , if Wire.str "path" entry == "" then text "" else code [ class "message-path" ] [ text (Wire.str "path" entry) ]
        ]



-- WORK


workView : Model -> Document -> Html Msg
workView model doc =
    if model.focus == Nothing then
        section [ class "panel" ]
            [ div [ class "panel-heading" ]
                [ div [] [ h2 [] [ text "Where time goes" ], p [] [ text ("Elapsed time by kind of work across " ++ (if optionalCount "activity_session_count" doc.root == "—" then "" else optionalCount "activity_session_count" doc.root ++ " ") ++ "recent sessions; nested work overlaps its parent. Choose a session above to see how a prompt branched into work.") ] ]
                , label [ class "inline-control" ] [ span [ class "muted" ] [ text "Show" ], select [ onInput DurationMode, attribute "aria-label" "Duration aggregation" ] [ choice model.durationMode "average" "Average per activity", choice model.durationMode "total" "Total time" ] ]
                ]
            , chart "summary-bars"
            ]

    else
        sessionView model doc


sessionView : Model -> Document -> Html Msg
sessionView model doc =
    let
        root = Wire.field "selected_session" doc.root
        nodes = waterfallNodes doc
        picked = model.selected |> Maybe.andThen (\nodeId -> List.filter (\node -> node.id == nodeId) nodes |> List.head)
        started = Wire.str "started_at" root
        caption = [ if started == "" then "" else "Started " ++ timeText started, Wire.str "state" root ] |> List.filter ((/=) "") |> String.join " · "
    in
    if List.isEmpty nodes then
        guidance "Session unavailable" "This session was not returned for the selected project."
    else
        panel (Wire.str "title" root) caption
            [ div [ class "graph-controls" ] [ div [ class "legend" ] (List.map (\( css, legendCaption ) -> span [ class css ] [ span [ class "legend-swatch", attribute "aria-hidden" "true" ] [], text legendCaption ]) [ ( "agent", "Subagent" ), ( "test", "Test" ), ( "script", "Script" ), ( "review", "Review" ), ( "tool", "Tool / command" ), ( "session", "Prompt / session" ) ]), div [] [ button [ onClick (Zoom 0.7), attribute "aria-label" "Zoom out" ] [ Icon.view Icon.ZoomOut ], button [ onClick (Zoom 1.4), attribute "aria-label" "Zoom in" ] [ Icon.view Icon.ZoomIn ] ] ]
            , div [ class "waterfall-scroll", attribute "tabindex" "0", on "keydown" (D.field "key" D.string |> D.map Keyboard), attribute "aria-label" "Session work tree. Arrow keys select tasks, parents and children." ] [ chart "work-waterfall" ]
            , p [ class "selection-line", attribute "aria-live" "polite" ] [ text (picked |> Maybe.map Waterfall.summary |> Maybe.withDefault "Time flows downward; work branches right from the prompt that started it. Repeated calls are grouped. Select a task to see its time and outcome.") ]
            ]



-- EVALUATIONS


evalView : Model -> Document -> Html Msg
evalView model doc =
    div []
        [ div [ class "stats-grid" ] [ stat "Recorded runs" (countText "run_count" doc.root) "Caller-owned execution", stat "Results" (countText "result_count" doc.root) "Current observations; corrections retained", stat "Agents" (String.fromInt (List.length (Dict.get "groups" model.supplemental |> Maybe.map .rows |> Maybe.withDefault doc.rows))) "Evaluated agents and configurations", stat "Source records" (countText "scanned_records" doc.root) "Bounded historical projection" ]
        , if model.focus == Nothing then
            guidance "Choose an agent" "Pick an agent above to see each of its metrics over time."
          else if List.isEmpty doc.metrics then
            guidance "No metrics recorded" "This agent has runs but no scored metric observations yet."
          else
            div [ class "metric-grid" ] (List.indexedMap metricPanel doc.metrics)
        ]


metricPanel : Int -> Wire.Metric -> Html Msg
metricPanel index metric =
    panel metric.label (String.join " · " (List.filter ((/=) "") [ metric.unit, metric.description ]))
        [ chart ("metric-" ++ String.fromInt index)
        , metricDefinition metric.calculation
        , div [ class "result-values" ] (List.reverse metric.points |> List.take 6 |> List.map (\pt -> div [ class "result-value" ] [ small [] [ timeView pt.at ], strongText pt.exact ]))
        ]


{-| Calculation descriptions are often structured JSON text; show their parts as readable sentences. -}
metricDefinition : String -> Html Msg
metricDefinition calculation =
    let
        captions = [ ( "formula", "Formula" ), ( "population", "Population" ), ( "aggregation", "Aggregation" ), ( "exclusions", "Exclusions" ), ( "missing_data", "Missing data" ) ]
    in
    if String.trim calculation == "" then
        text ""
    else
        case D.decodeString (D.keyValuePairs D.value) calculation of
            Ok fields ->
                let
                    parts = captions |> List.filterMap (\( key, caption ) -> List.filter (\( k, _ ) -> k == key) fields |> List.head |> Maybe.andThen (\( _, item ) -> D.decodeValue D.string item |> Result.toMaybe) |> Maybe.map (\content -> p [] [ Html.strong [] [ text (caption ++ ". ") ], text content ]))
                in
                if List.isEmpty parts then text "" else div [ class "metric-definition" ] (h3 [] [ text "How it is calculated" ] :: parts)

            Err _ ->
                div [ class "metric-definition" ] [ h3 [] [ text "How it is calculated" ], p [] [ text calculation ] ]



-- TOOLS AND SKILLS


toolsView : Model -> Document -> Html Msg
toolsView model doc =
    case model.query.kind of
        "analytics_summary" ->
            div []
                [ div [ class "stats-grid" ] [ stat "Observed calls" (countText "observed_calls" doc.root) "Invocations may contain several operations", stat "Identified operations" (countText "identified_actions" doc.root) "Semantic activity classification", stat "Unclassified" (countText "unclassified_actions" doc.root) "Coverage remains explicit", stat "Window" "7 days" "Rolling observation window" ]
                , panel "What agents are doing" "Operations by kind of activity. Open one to see which commands it was, over time."
                    [ barList model (List.map (\row -> ( { key = commandsKey row.id, load = query "analytics_commands" |> with "category_id" row.id, icon = Icon.Tools, title = row.title, subtitle = row.subtitle, aside = text "" }, Wire.number "actions" row.body )) doc.rows) commandBreakdown ]
                ]

        "analytics_examples" ->
            panel "Observed implementations" "" [ div [ class "record-list" ] (List.map (\row -> plainRow Icon.Tools row.title row.subtitle) doc.rows) ]

        _ ->
            panel (if model.mode == "opportunities" then "Opportunities" else "Capabilities") (if model.mode == "opportunities" then "Recurring improvised work that could deserve a supplied tool." else "Supplied tools and skills and how often they were used.")
                [ div [ class "record-list" ] (List.map (\row -> button [ class "record-row", onClick (Navigate (if row.kind == "capability" then query "analytics_examples" |> with "resource_key" row.id else query "analytics_examples" |> with "category_id" row.id)) ] [ Icon.view Icon.Tools, div [ class "record-copy" ] [ strongText row.title, small [] [ text row.subtitle ] ], span [ class "row-arrow" ] [ Icon.view Icon.Chevron ] ]) doc.rows) ]


commandBreakdown : Int -> Maybe Document -> Html Msg
commandBreakdown category loaded =
    case loaded of
        Nothing -> text ""
        Just doc ->
            let
                commands = Wire.list "commands" doc.root
            in
            if List.isEmpty commands then
                nothingRecorded "No commands were recorded for this activity in the window."
            else
                div [ class "command-grid" ]
                    (List.indexedMap
                        (\position command ->
                            div [ class "command" ] [ div [ class "command-heading" ] [ code [] [ text (Wire.str "name" command) ], span [ class "bar-value" ] [ text (countText "count" command ++ " runs") ] ], chart (commandChartId category position) ]
                        )
                        commands
                        ++ [ p [ class "chart-note" ] [ text ("Runs per " ++ duration (Wire.number "bin_seconds" doc.root) ++ " across the last seven days.") ] ]
                    )


skillsView : Model -> Document -> Html Msg
skillsView model doc =
    let
        skills = List.filter (\row -> row.kind == "skill") doc.rows
    in
    panel "Skill usage" "Named skills across the rolling seven-day window. Open a skill to read it."
        [ barList model (List.map (\row -> ( { key = "skill:" ++ row.id, load = query "skill_get" |> with "name" row.id, icon = Icon.Book, title = row.title, subtitle = row.subtitle, aside = text "" }, Wire.number "count" row.body )) skills) (\_ -> skillText) ]


skillText : Maybe Document -> Html Msg
skillText loaded =
    case loaded of
        Nothing -> text ""
        Just doc ->
            let
                skill = doc.root
                content = Wire.str "content" skill
            in
            div [ class "skill-text" ]
                [ if Wire.str "description" skill == "" then text "" else p [ class "message-headline" ] [ text (Wire.str "description" skill) ]
                , if content == "" then nothingRecorded "Nexus has not received this skill's text yet. Agents running an updated Nexus client send it the next time they read the skill." else pre [ class "document-text" ] [ text content ]
                , if Wire.str "source_path" skill == "" then text "" else small [] [ text (Wire.str "source_path" skill), text " · ", timeView (Wire.str "observed_at" skill) ]
                ]



-- CODE CONCERNS, PAPERCUTS, SOS


concernsView : Model -> Document -> Html Msg
concernsView model doc =
    if model.query.kind == "concern_dashboard" then
        let
            counts = Wire.field "counts" doc.root
        in
        div []
            [ div [ class "stats-grid" ] [ stat "Reported concerns" (countText "concerns" counts) "No reports does not establish code health", stat "Affected work sessions" (if Wire.number "work_sessions" counts == 0 && Wire.number "unknown_work_observations" counts > 0 then "Unknown" else countText "work_sessions" counts) "Reports must name their work session to count", stat "Source occurrences" (countText "known_source_occurrences" counts) "Known failure encounters", stat "Returns after resolution" (countText "returns_after_resolution" counts) "Potential recurring problem areas" ]
            , panel "Problem areas" "Reported concerns by code area. Select an area to read its concerns." [ chart "summary-bars" ]
            , panel "Reports over time" "Reports per UTC day." [ chart "summary-trend" ]
            ]

    else
        panel "Concerns" "Open a concern to read its encounters and the remedies already tried."
            [ if List.isEmpty doc.rows then nothingRecorded "No concerns were reported for this scope." else div [ class "record-list" ] (List.map (\row -> expandable model { key = "concern:" ++ row.id, load = query "concern_get" |> with "concern_id" row.id, icon = Icon.Bug, title = row.title, subtitle = row.subtitle, aside = text "" } concernDetail) doc.rows) ]


concernDetail : Maybe Document -> Html Msg
concernDetail loaded =
    case loaded of
        Nothing -> text ""
        Just doc ->
            let
                record = Wire.field "concern" doc.root
                history = Wire.list "history" record
            in
            div []
                [ proseView (Wire.field "summary" record)
                , if List.isEmpty history then text "" else reportList (List.map (\observation -> let input_ = Wire.field "input" observation in ( String.replace "_" " " (Wire.str "event" (Wire.field "body" input_)), Wire.str "observed_at" input_, Wire.field "body" input_ )) history)
                ]


{-| Individual reports in order, each with who reported it and when. -}
reportList : List ( String, String, D.Value ) -> Html Msg
reportList reports =
    div [ class "report-list" ]
        (List.map
            (\( who, at, body ) ->
                div [ class "report" ] [ header [] [ span [ class "message-from" ] [ text who ], small [] [ timeView at ] ], proseView body, if prose body == [] && Wire.str "title" body /= "" then p [] [ text (Wire.str "title" body) ] else text "" ]
            )
            reports
        )


papercutsView : Model -> Document -> Html Msg
papercutsView model doc =
    panel "Papercuts" "Most encountered first, then most recent. Open one to read every report."
        [ if List.isEmpty doc.rows then nothingRecorded "No open papercuts were reported for this scope." else div [ class "record-list" ] (List.map (\row -> expandable model { key = "papercut:" ++ row.id, load = query "papercut_get" |> with "papercut_id" row.id, icon = Icon.Warning, title = row.title, subtitle = row.subtitle, aside = text "" } papercutDetail) doc.rows) ]


papercutDetail : Maybe Document -> Html Msg
papercutDetail loaded =
    case loaded of
        Nothing -> text ""
        Just doc ->
            let
                reports = Wire.list "reports" doc.root
                papercut = Wire.field "papercut" doc.root
            in
            if List.isEmpty reports then proseView papercut else reportList (List.map (\report -> ( reporter report, Wire.str "created_at" report, report )) reports)


reporter : D.Value -> String
reporter report =
    [ Wire.str "reporter_agent" report, Wire.str "creator_agent" report, Wire.str "agent" (Wire.field "reporter" report), Wire.str "agent" report, Wire.str "source" report ] |> List.filter ((/=) "") |> List.head |> Maybe.withDefault "Report"


sosView : Model -> Document -> Html Msg
sosView model doc =
    div []
        [ div [ class "stats-grid two" ] [ stat "Reported issues" (String.fromInt (List.length doc.rows)) "Ranked by reports, then latest arrival", stat "Untriaged reports" (countText "untriaged_report_count" doc.root) "Source reports awaiting a triage decision" ]
        , panel "Situations" "Open one to read every report."
            [ if List.isEmpty doc.rows then nothingRecorded "No situations were reported for this scope." else div [ class "record-list" ] (List.map (\row -> expandable model { key = "sos:" ++ row.id, load = query "sos_get" |> with "issue_id" row.id, icon = Icon.Siren, title = row.title, subtitle = row.subtitle, aside = text "" } sosDetail) doc.rows) ]
        ]


sosDetail : Maybe Document -> Html Msg
sosDetail loaded =
    case loaded of
        Nothing -> text ""
        Just doc ->
            let
                detail = Wire.field "detail" doc.root
                reports = Wire.list "reports" detail
            in
            div [] [ proseView (Wire.field "issue" detail), reportList (List.map (\report -> ( reporter report, defaultText (Wire.str "received_at" report) (Wire.str "created_at" report), report )) reports) ]


defaultText : String -> String -> String
defaultText fallback value_ =
    if value_ == "" then fallback else value_



-- MEMORY


memoryView : Model -> Document -> Html Msg
memoryView model doc =
    div []
        [ Dict.get "health" model.supplemental |> Maybe.map (.root >> Wire.field "health" >> memoryHealth) |> Maybe.withDefault (text "")
        , panel "Knowledge" "Notes agents recorded and the summaries built from them. Each level summarizes twice as many notes as the one below it; open a summary to see what it was built from."
            [ if List.isEmpty doc.rows then nothingRecorded "No memories were recorded for this project." else div [ class "record-list" ] (List.map (memoryRow model 0) doc.rows) ]
        ]


memoryHealth : D.Value -> Html Msg
memoryHealth health =
    let
        scopes = Wire.list "scopes" health
        total key = scopes |> List.map (Wire.number key) |> List.sum
        providers = Wire.list "providers" health
        providerText provider = Wire.str "provider" provider ++ (if Wire.str "last_outcome" provider == "failed" then " failing (" ++ countText "consecutive_failures" provider ++ " in a row)" else if Wire.str "last_outcome" provider == "" then " idle" else " " ++ Wire.str "last_outcome" provider)
    in
    div [ class "stats-grid" ]
        [ stat "Notes" (formatCount (total "raw_entries")) "Raw entries agents recorded"
        , stat "Summaries" (formatCount (total "summaries")) "Built by pairwise aggregation"
        , stat "Pending" (countText "pending_summaries" health) "Summaries waiting to be built"
        , stat "Summarizers" (if List.isEmpty providers then "—" else String.fromInt (List.length (List.filter (\provider -> Wire.str "last_outcome" provider /= "failed") providers)) ++ "/" ++ String.fromInt (List.length providers) ++ " healthy") (String.join " · " (List.map providerText providers))
        ]


memoryRow : Model -> Int -> Row -> Html Msg
memoryRow model depth row =
    let
        level = round (Wire.number "level" row.body)
        span_ = round (Wire.number "span" row.body)
        isSummary = row.kind == "summary" || level > 0
        badge = levelBadge level span_ isSummary
    in
    expandable model { key = "memory:" ++ row.id, load = if isSummary && depth < 8 then query "memory_expand" |> with "summary_id" row.id else query "", icon = Icon.Memory, title = row.title, subtitle = timeText row.subtitle, aside = badge } (memoryDetail model depth row)


levelBadge : Int -> Int -> Bool -> Html Msg
levelBadge level span_ isSummary =
    let
        caption = if isSummary then "Level " ++ String.fromInt level ++ (if span_ > 0 then " · " ++ String.fromInt span_ ++ " notes" else "") else "Note"
    in
    span [ class "level", attribute "title" caption ] [ span [ class "level-pips", attribute "aria-hidden" "true" ] (List.repeat (min 12 level) (span [ class "pip" ] [])), text caption ]


memoryDetail : Model -> Int -> Row -> Maybe Document -> Html Msg
memoryDetail model depth row loaded =
    let
        sources = loaded |> Maybe.map .rows |> Maybe.withDefault [] |> List.filter (\source -> source.id /= row.id)
    in
    div []
        [ pre [ class "document-text" ] [ text (Wire.str "content" row.body) ]
        , if List.isEmpty sources then text "" else div [ class "memory-sources" ] (h3 [] [ text "Built from" ] :: List.map (memoryRow model (depth + 1)) sources)
        ]


pagination : Model -> Document -> Html Msg
pagination model doc =
    let
        hasMore = doc.next /= Nothing || (D.decodeValue (D.field "next_offset" (D.nullable D.int)) doc.root |> Result.withDefault Nothing) /= Nothing || model.query.kind == "papercuts" && D.decodeValue (D.field "truncated" D.bool) doc.root == Ok True
    in
    if hasMore then div [ class "pagination" ] [ button [ onClick More, disabled model.data.loading ] [ text "Show more →" ] ] else text ""
