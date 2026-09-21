-- @feature observability-ui
-- @spec docs/features/observability-ui.md
-- @entrypoint main
-- @boundary elm-port-json
port module Main exposing (main)

import Browser
import Browser.Events
import Html exposing (Html, aside, button, div, h1, h2, h3, header, main_, option, p, section, select, span, text)
import Html.Attributes exposing (attribute, class, id, selected, tabindex, value)
import Html.Events exposing (onClick, onInput)
import Http
import Iso8601
import Json.Decode as Decode
import Json.Encode as Encode
import Nexus.Api as Api
import Nexus.Domain exposing (Bucket, Claim, Conflict, Dashboard, Event, EventClass(..), ProjectScope(..), ProjectSummary, Series, Session, bucketEvents)
import Nexus.Polling as Polling exposing (DataState(..))
import Time exposing (Posix)


port renderActivityChart : Encode.Value -> Cmd message


type alias Model =
    { projects : List ProjectSummary
    , scope : ProjectScope
    , dashboard : DataState Dashboard
    , selectedDetail : Maybe Detail
    }


type Detail
    = EventDetail Event
    | SessionDetail Session
    | ClaimDetail Claim
    | ConflictDetail Conflict


type Msg
    = GotProjects (Result HttpError (List ProjectSummary))
    | GotDashboard ProjectScope (Result HttpError Dashboard)
    | Tick Posix
    | ScopeChanged String
    | SelectDetail Detail
    | CloseDetail
    | KeyPressed String


type alias HttpError =
    Http.Error


main : Program () Model Msg
main =
    Browser.element
        { init = init
        , update = update
        , subscriptions = subscriptions
        , view = view
        }


init : () -> ( Model, Cmd Msg )
init _ =
    ( { projects = []
      , scope = AllProjects
      , dashboard = Loading
      , selectedDetail = Nothing
      }
    , Cmd.batch [ Api.fetchProjects GotProjects, Api.fetchDashboard AllProjects (GotDashboard AllProjects) ]
    )


update : Msg -> Model -> ( Model, Cmd Msg )
update message model =
    case message of
        GotProjects result ->
            ( { model | projects = Result.withDefault model.projects result }, Cmd.none )

        GotDashboard responseScope result ->
            if Polling.isCurrentScope model.scope responseScope then
                case result of
                    Ok dashboard ->
                        ( { model | dashboard = Polling.succeed dashboard model.dashboard }
                        , renderActivityChart (encodeBuckets (bucketEvents dashboard.generatedAt dashboard.events.items))
                        )

                    Err error ->
                        ( { model | dashboard = Polling.fail (Api.httpError error) model.dashboard }, Cmd.none )

            else
                ( model, Cmd.none )

        Tick _ ->
            if Polling.canPoll model.dashboard then
                ( { model | dashboard = Polling.begin model.dashboard }
                , Api.fetchDashboard model.scope (GotDashboard model.scope)
                )

            else
                ( model, Cmd.none )

        ScopeChanged projectId ->
            let
                scope =
                    if projectId == "" then
                        AllProjects

                    else
                        OneProject projectId
            in
            ( { model | scope = scope, dashboard = Polling.begin model.dashboard, selectedDetail = Nothing }
            , Api.fetchDashboard scope (GotDashboard scope)
            )

        SelectDetail detail ->
            ( { model | selectedDetail = Just detail }, Cmd.none )

        CloseDetail ->
            ( { model | selectedDetail = Nothing }, Cmd.none )

        KeyPressed key ->
            if key == "Escape" then
                ( { model | selectedDetail = Nothing }, Cmd.none )

            else
                ( model, Cmd.none )


subscriptions : Model -> Sub Msg
subscriptions model =
    Sub.batch
        [ Time.every (toFloat (refreshInterval model)) Tick
        , Browser.Events.onKeyDown (Decode.map KeyPressed (Decode.field "key" Decode.string))
        ]


refreshInterval : Model -> Int
refreshInterval model =
    model.dashboard
        |> Polling.value
        |> Maybe.map .refreshIntervalMs
        |> Maybe.withDefault 2000


view : Model -> Html Msg
view model =
    div [ class "shell" ]
        [ header [ class "topbar" ]
            [ div []
                [ p [ class "eyebrow" ] [ text "ADVISORY COORDINATION" ]
                , h1 [] [ text "Nexus" ]
                ]
            , div [ class "topbar-controls" ]
                [ projectPicker model
                , statusPill model.dashboard
                ]
            ]
        , staleBanner model.dashboard
        , main_ [ class "dashboard" ] [ dashboardView model ]
        , detailDrawer model.selectedDetail
        ]


projectPicker : Model -> Html Msg
projectPicker model =
    div [ class "project-picker" ]
        [ span [ class "control-label" ] [ text "Repository" ]
        , select [ onInput ScopeChanged, attribute "aria-label" "Filter repository" ]
            (option [ value "", selected (model.scope == AllProjects) ] [ text "All projects" ]
                :: List.map (projectOption model.scope) model.projects
            )
        ]


projectOption : ProjectScope -> ProjectSummary -> Html Msg
projectOption scope project =
    option
        [ value project.projectId
        , selected (scope == OneProject project.projectId)
        ]
        [ text (projectLabel project) ]


projectLabel : ProjectSummary -> String
projectLabel project =
    let
        directory =
            project.worktrees
                |> List.head
                |> Maybe.andThen (String.split "/" >> List.reverse >> List.head)
                |> Maybe.withDefault "project"
    in
    directory ++ " · " ++ String.left 8 project.projectId


statusPill : DataState Dashboard -> Html Msg
statusPill state =
    let
        ( label, modifier ) =
            case state of
                Loading ->
                    ( "Connecting", "pending" )

                Refreshing _ ->
                    ( "Refreshing", "pending" )

                Fresh _ ->
                    ( "Live", "live" )

                Stale _ _ ->
                    ( "Stale", "stale" )

                Failed _ ->
                    ( "Offline", "stale" )
    in
    span [ class ("status-pill " ++ modifier) ] [ span [ class "status-dot" ] [], text label ]


staleBanner : DataState Dashboard -> Html Msg
staleBanner state =
    case state of
        Stale _ message ->
            div [ class "banner", attribute "role" "status" ] [ text (message ++ " Showing the last successful snapshot.") ]

        Failed message ->
            div [ class "banner error", attribute "role" "alert" ] [ text message ]

        _ ->
            text ""


dashboardView : Model -> Html Msg
dashboardView model =
    case Polling.value model.dashboard of
        Nothing ->
            section [ class "loading-panel" ] [ h2 [] [ text "Connecting to Nexus" ], p [] [ text "Waiting for the first coordination snapshot…" ] ]

        Just dashboard ->
            div []
                [ countStrip dashboard
                , activityPanel dashboard
                , div [ class "operations-grid" ]
                    [ conflictPanel dashboard.conflicts.items
                    , sessionPanel dashboard.sessions.items
                    , claimPanel dashboard.claims.items
                    ]
                , eventPanel dashboard.events.items dashboard.events.truncated
                ]


countStrip : Dashboard -> Html Msg
countStrip dashboard =
    section [ class "count-strip", attribute "aria-label" "Nexus status" ]
        [ countCard "Active sessions" dashboard.counts.activeSessions
        , countCard "Active claims" dashboard.counts.activeClaims
        , countCard "Open conflicts" dashboard.counts.openConflicts
        , countCard "Recorded events" dashboard.counts.events
        ]


countCard : String -> Int -> Html Msg
countCard label number =
    div [ class "count-card" ]
        [ span [ class "count-value" ] [ text (String.fromInt number) ]
        , span [ class "count-label" ] [ text label ]
        ]


activityPanel : Dashboard -> Html Msg
activityPanel dashboard =
    section [ class "panel activity-panel" ]
        [ panelHeading "Recent activity" "Five-minute event buckets · last hour"
        , div [ class "chart-legend" ]
            [ legend "Activity" "activity"
            , legend "Info" "info"
            , legend "Warning" "warning"
            , legend "Critical" "critical"
            ]
        , div [ id "activity-chart", class "chart" ] []
        , if dashboard.events.truncated then
            p [ class "truncation" ] [ text "Chart uses the latest bounded event window." ]

          else
            text ""
        ]


legend : String -> String -> Html Msg
legend label modifier =
    span [ class "legend-item" ] [ span [ class ("legend-swatch " ++ modifier) ] [], text label ]


conflictPanel : List Conflict -> Html Msg
conflictPanel conflicts =
    section [ class "panel" ]
        [ panelHeading "Conflicts" "Open items first"
        , recordList
            (conflicts
                |> List.sortBy (\conflict -> ( closedOrder conflict.status, severityOrder conflict.severity ))
                |> List.map conflictRow
            )
            "No conflicts recorded."
        ]


conflictRow : Conflict -> Html Msg
conflictRow conflict =
    recordButton (ConflictDetail conflict)
        [ span [ class ("severity " ++ conflict.severity) ] [ text conflict.severity ]
        , div [ class "record-copy" ]
            [ span [ class "record-title" ] [ text (shortPath conflict.path) ]
            , span [ class "record-meta" ] [ text (conflict.kind ++ " · " ++ conflict.status) ]
            ]
        ]


sessionPanel : List Session -> Html Msg
sessionPanel sessions =
    section [ class "panel" ]
        [ panelHeading "Sessions" "Most recently seen"
        , recordList (List.map sessionRow sessions) "No agent sessions recorded."
        ]


sessionRow : Session -> Html Msg
sessionRow session =
    recordButton (SessionDetail session)
        [ span [ class ("agent-mark " ++ session.status) ] [ text (String.left 1 (String.toUpper session.agent)) ]
        , div [ class "record-copy" ]
            [ span [ class "record-title" ] [ text (Maybe.withDefault session.sessionId session.taskSummary) ]
            , span [ class "record-meta" ] [ text (session.agent ++ " · " ++ session.status) ]
            ]
        ]


claimPanel : List Claim -> Html Msg
claimPanel claims =
    section [ class "panel" ]
        [ panelHeading "Active claims" "Advisory, never locking"
        , recordList (List.map claimRow claims) "No active path claims."
        ]


claimRow : Claim -> Html Msg
claimRow claim =
    recordButton (ClaimDetail claim)
        [ span [ class "operation" ] [ text claim.operation ]
        , div [ class "record-copy" ]
            [ span [ class "record-title" ] [ text (shortPath claim.path) ]
            , span [ class "record-meta" ] [ text (claim.sessionId ++ lineRange claim) ]
            ]
        ]


eventPanel : List Event -> Bool -> Html Msg
eventPanel events truncated =
    section [ class "panel event-panel" ]
        [ panelHeading "Event stream"
            (if truncated then
                "Newest bounded window"

             else
                "Newest first"
            )
        , recordList (List.map eventRow events) "No activity recorded yet."
        ]


eventRow : Event -> Html Msg
eventRow event =
    recordButton (EventDetail event)
        [ span [ class "event-kind" ] [ text (eventGlyph event.kind) ]
        , div [ class "record-copy" ]
            [ span [ class "record-title" ] [ text (humanize event.kind) ]
            , span [ class "record-meta" ] [ text (formatTime event.createdAt ++ " · " ++ String.left 8 event.projectId) ]
            ]
        ]


panelHeading : String -> String -> Html Msg
panelHeading title subtitle =
    div [ class "panel-heading" ] [ h2 [] [ text title ], span [] [ text subtitle ] ]


recordList : List (Html Msg) -> String -> Html Msg
recordList records empty =
    if List.isEmpty records then
        p [ class "empty" ] [ text empty ]

    else
        div [ class "record-list" ] records


recordButton : Detail -> List (Html Msg) -> Html Msg
recordButton detail contents =
    button [ class "record", onClick (SelectDetail detail) ] contents


detailDrawer : Maybe Detail -> Html Msg
detailDrawer selected =
    case selected of
        Nothing ->
            text ""

        Just detail ->
            aside
                [ class "detail-drawer"
                , attribute "role" "dialog"
                , attribute "aria-modal" "false"
                , attribute "aria-label" "Record details"
                , tabindex 0
                ]
                [ div [ class "drawer-heading" ]
                    [ div [] [ p [ class "eyebrow" ] [ text (detailKind detail) ], h3 [] [ text (detailTitle detail) ] ]
                    , button [ class "close", onClick CloseDetail, attribute "aria-label" "Close details" ] [ text "×" ]
                    ]
                , div [ class "detail-fields" ] (List.map detailField (detailFields detail))
                ]


detailField : ( String, String ) -> Html Msg
detailField ( label, fieldValue ) =
    div [] [ span [] [ text label ], p [] [ text fieldValue ] ]


detailKind : Detail -> String
detailKind detail =
    case detail of
        EventDetail _ ->
            "EVENT"

        SessionDetail _ ->
            "SESSION"

        ClaimDetail _ ->
            "CLAIM"

        ConflictDetail _ ->
            "CONFLICT"


detailTitle : Detail -> String
detailTitle detail =
    case detail of
        EventDetail event ->
            humanize event.kind

        SessionDetail session ->
            Maybe.withDefault session.sessionId session.taskSummary

        ClaimDetail claim ->
            shortPath claim.path

        ConflictDetail conflict ->
            shortPath conflict.path


detailFields : Detail -> List ( String, String )
detailFields detail =
    case detail of
        EventDetail event ->
            [ ( "Project", event.projectId )
            , ( "Session", Maybe.withDefault "—" event.sessionId )
            , ( "Time", Iso8601.fromTime event.createdAt )
            , ( "Payload", Encode.encode 2 event.payload )
            ]

        SessionDetail session ->
            [ ( "Session", session.sessionId )
            , ( "Agent", session.agent )
            , ( "Status", session.status )
            , ( "Project", session.projectId )
            , ( "Worktree", Maybe.withDefault "—" session.worktree )
            , ( "Last seen", Iso8601.fromTime session.lastSeenAt )
            ]

        ClaimDetail claim ->
            [ ( "Path", claim.path )
            , ( "Operation", claim.operation )
            , ( "State", claim.state )
            , ( "Session", claim.sessionId )
            , ( "Lines", lineRangeText claim.lineStart claim.lineEnd )
            , ( "Expires", Iso8601.fromTime claim.expiresAt )
            ]

        ConflictDetail conflict ->
            [ ( "Path", conflict.path )
            , ( "Severity", conflict.severity )
            , ( "Status", conflict.status )
            , ( "Kind", conflict.kind )
            , ( "Message", conflict.message )
            , ( "Updated", Iso8601.fromTime conflict.updatedAt )
            ]


encodeBuckets : List Bucket -> Encode.Value
encodeBuckets buckets =
    Encode.list encodeBucket buckets


encodeBucket : Bucket -> Encode.Value
encodeBucket bucket =
    Encode.object
        [ ( "startMilliseconds", Encode.int (Time.posixToMillis bucket.start) )
        , ( "series", Encode.list encodeSeries bucket.series )
        ]


encodeSeries : Series -> Encode.Value
encodeSeries item =
    Encode.object
        [ ( "class", Encode.string (eventClassName item.class) )
        , ( "count", Encode.int item.count )
        ]


eventClassName : EventClass -> String
eventClassName eventClass =
    case eventClass of
        Activity ->
            "activity"

        Info ->
            "info"

        Warning ->
            "warning"

        Critical ->
            "critical"


severityOrder : String -> Int
severityOrder severity =
    case severity of
        "critical" ->
            0

        "warning" ->
            1

        _ ->
            2


closedOrder : String -> Int
closedOrder status =
    if status == "open" then
        0

    else
        1


lineRange : Claim -> String
lineRange claim =
    case ( claim.lineStart, claim.lineEnd ) of
        ( Just first, Just last ) ->
            " · L" ++ String.fromInt first ++ "–" ++ String.fromInt last

        _ ->
            ""


lineRangeText : Maybe Int -> Maybe Int -> String
lineRangeText first last =
    case ( first, last ) of
        ( Just start, Just end ) ->
            String.fromInt start ++ "–" ++ String.fromInt end

        _ ->
            "Unknown range"


shortPath : String -> String
shortPath path =
    path
        |> String.split "/"
        |> List.reverse
        |> List.take 3
        |> List.reverse
        |> String.join "/"


formatTime : Posix -> String
formatTime time =
    Iso8601.fromTime time |> String.slice 11 19


humanize : String -> String
humanize value =
    value
        |> String.split "_"
        |> List.map capitalize
        |> String.join " "


capitalize : String -> String
capitalize value =
    String.toUpper (String.left 1 value) ++ String.dropLeft 1 value


eventGlyph : String -> String
eventGlyph kind =
    if String.startsWith "conflict" kind then
        "!"

    else if String.startsWith "tool" kind then
        "↗"

    else
        "·"
