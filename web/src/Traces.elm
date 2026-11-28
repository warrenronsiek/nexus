-- @feature observability-ui
-- @spec docs/features/observability-ui.md
-- @boundary browser-read: format scoped observations and sensitive text as inert HTML.


module Traces exposing (Scope, ScopeField, controls, empty, fields, pageFields, parseScope, query, referenceScope, setScopeField, showContent, view)

import Dict exposing (Dict)
import Html exposing (Html, a, button, code, div, h2, h3, label, option, p, pre, section, select, small, span, text)
import Html.Attributes exposing (attribute, class, href, selected, value)
import Html.Events exposing (onClick, onInput)
import Icon
import Json.Decode as D
import Json.Encode as E
import TraceFlow
import Set
import Wire exposing (Document)


type alias Scope =
    { application : String
    , environment : String
    , agent : String
    , trace : String
    , event : String
    , mode : String
    , name : String
    , version : Version
    }


type Version
    = AnyVersion
    | ExactVersion String
    | Unversioned


empty : Scope
empty =
    Scope "" "" "" "" "" "" "" AnyVersion


parseScope : Dict String String -> Scope
parseScope params =
    let
        read key =
            Dict.get key params |> Maybe.withDefault ""

        mode =
            read "view"
    in
    Scope (read "app")
        (read "env")
        (read "agent")
        (read "trace")
        (read "event")
        (if List.member mode [ "tool", "skill" ] then
            mode

         else
            ""
        )
        (read "name")
        (case Dict.get "version" params of
            Just version ->
                ExactVersion version

            Nothing ->
                if read "unversioned" == "1" then
                    Unversioned

                else
                    AnyVersion
        )


fields : Scope -> List ( String, String )
fields scope =
    nonempty
        [ ( "app", scope.application )
        , ( "env", scope.environment )
        , ( "agent", scope.agent )
        , ( "trace", scope.trace )
        , ( "event", scope.event )
        , ( "view", scope.mode )
        , ( "name", scope.name )
        ]
        ++ (case scope.version of
                AnyVersion ->
                    []

                ExactVersion version ->
                    [ ( "version", version ) ]

                Unversioned ->
                    [ ( "unversioned", "1" ) ]
           )


query : Scope -> { kind : String, fields : List ( String, E.Value ) }
query scope =
    if scope.trace /= "" && scope.application /= "" && scope.environment /= "" then
        { kind = "trace_get"
        , fields =
            strings
                [ ( "application_id", scope.application ), ( "environment", scope.environment ), ( "trace_id", scope.trace ), ( "event_id", scope.event ) ]
                ++ [ ( "limit", E.int 64 ) ]
        }

    else
        { kind =
            if scope.mode == "" then
                "trace_list"

            else if scope.name == "" then
                "trace_usage"

            else
                "trace_calls"
        , fields =
            [ ( "filter"
              , E.object
                    (strings
                        [ ( "application_id", scope.application )
                        , ( "environment", scope.environment )
                        , ( "agent_id", scope.agent )
                        , ( "kind", scope.mode )
                        , ( "name", scope.name )
                        ]
                        ++ versionFilter scope.version
                    )
              )
            ]
        }


versionFilter : Version -> List ( String, E.Value )
versionFilter version =
    case version of
        AnyVersion ->
            []

        ExactVersion value_ ->
            [ ( "version", E.object (strings [ ( "kind", "exact" ), ( "value", value_ ) ]) ) ]

        Unversioned ->
            [ ( "version", E.object (strings [ ( "kind", "unversioned" ) ]) ) ]


pageFields : String -> E.Value -> List ( String, E.Value ) -> List ( String, E.Value )
pageFields kind cursor active =
    if kind == "trace_get" then
        ( "cursor", cursor ) :: List.filter (Tuple.first >> (/=) "cursor") active

    else
        let
            filter =
                active |> List.head |> Maybe.map Tuple.second |> Maybe.andThen (D.decodeValue (D.keyValuePairs D.value) >> Result.toMaybe) |> Maybe.withDefault []
        in
        [ ( "filter", E.object (( "cursor", cursor ) :: List.filter (Tuple.first >> (/=) "cursor") filter) ) ]


showContent : Bool -> List ( String, E.Value ) -> List ( String, E.Value )
showContent enabled active =
    ( "include_payload", E.bool enabled ) :: List.filter (Tuple.first >> (/=) "include_payload") active


strings : List ( String, String ) -> List ( String, E.Value )
strings pairs =
    nonempty pairs |> List.map (Tuple.mapSecond E.string)


nonempty : List ( String, String ) -> List ( String, String )
nonempty =
    List.filter (Tuple.second >> (/=) "")


type ScopeField
    = Application
    | Environment
    | Agent


setScopeField : ScopeField -> String -> Scope -> Scope
setScopeField field picked scope =
    case field of
        Application ->
            { scope | application = picked, environment = "", agent = "" }

        Environment ->
            { scope | environment = picked, agent = "" }

        Agent ->
            { scope | agent = picked }


controls : (ScopeField -> String -> msg) -> Scope -> Maybe Document -> List (Html msg)
controls change scope status =
    if scope.trace /= "" then
        []

    else
        let
            scopes =
                status |> Maybe.map (.root >> Wire.list "scopes") |> Maybe.withDefault []

            same field current item =
                current == "" || current == Wire.str field item

            applications =
                scopes

            environments =
                List.filter (same "application_id" scope.application) scopes

            agents =
                List.filter (same "environment" scope.environment) environments

            picker caption placeholder key current field items =
                let
                    choices =
                        Set.fromList (current :: List.map (Wire.str key) items) |> Set.remove "" |> Set.toList

                    choose id label_ =
                        option [ value id, selected (id == current) ] [ text label_ ]
                in
                label [ class "scope-control" ]
                    [ span [ class "muted" ] [ text caption ]
                    , select [ onInput (change field), attribute "aria-label" caption ] (choose "" placeholder :: List.map (\id -> choose id id) choices)
                    ]
        in
        [ picker "Application" "All applications" "application_id" scope.application Application applications
        , picker "Environment" "All environments" "environment" scope.environment Environment environments
        , picker "Agent" "All agents" "agent_id" scope.agent Agent agents
        ]


referenceScope : D.Value -> Scope
referenceScope reference =
    { empty | application = Wire.str "application_id" reference, environment = Wire.str "environment" reference, trace = Wire.str "trace_id" reference, event = Wire.str "event_id" reference }


view : (Scope -> String) -> Scope -> String -> { allow : Bool, shown : Bool, toggle : msg } -> Maybe Document -> Document -> Html msg
view linkTo scope kind content status document =
    case kind of
        "trace_get" ->
            detail linkTo scope content (Wire.list "events" document.root)

        "trace_usage" ->
            div [] [ tabs linkTo scope, pending status, panel (capabilityTitle scope ++ " usage") (List.map (usageRow linkTo scope) (Wire.list "usage" document.root)) "No calls were recorded in this scope." ]

        "trace_calls" ->
            div [] [ tabs linkTo scope, pending status, panel (scope.name ++ " calls") (List.map (callRow linkTo scope) (Wire.list "calls" document.root)) "No calls were recorded in this scope." ]

        _ ->
            div [] [ tabs linkTo scope, pending status, panel "Recent traces" (List.map (traceRow linkTo scope) (Wire.list "traces" document.root)) "No application traces were recorded in this scope." ]


tabs : (Scope -> String) -> Scope -> Html msg
tabs linkTo scope =
    div [ class "subnav", attribute "aria-label" "Application trace views" ]
        (List.map
            (\( mode, caption ) ->
                a
                    [ href (linkTo { scope | mode = mode, name = "", version = AnyVersion })
                    , class
                        (if scope.mode == mode then
                            "active"

                         else
                            ""
                        )
                    , attribute "aria-current"
                        (if mode == scope.mode then
                            "page"

                         else
                            "false"
                        )
                    ]
                    [ text caption ]
            )
            [ ( "", "Traces" ), ( "skill", "Skills" ), ( "tool", "Tools" ) ]
        )


pending : Maybe Document -> Html msg
pending status =
    case status of
        Nothing ->
            text ""

        Just document ->
            let
                batches =
                    Wire.number "pending_batches" document.root

                truncated =
                    D.decodeValue (D.field "scopes_truncated" D.bool) document.root == Ok True

                messages =
                    (if batches > 0 then
                        [ count "pending_batches" document.root
                            ++ (if batches == 1 then
                                    " trace batch awaits processing in this project."

                                else
                                    " trace batches await processing in this project."
                               )
                        ]

                     else
                        []
                    )
                        ++ (if truncated then
                                [ "Some application scopes are omitted from the pickers." ]

                            else
                                []
                           )
            in
            if List.isEmpty messages then
                text ""

            else
                p [ class "trace-status", attribute "role" "status" ] [ text (String.join " " messages) ]


capabilityTitle : Scope -> String
capabilityTitle scope =
    if scope.mode == "skill" then
        "Skill"

    else
        "Tool"


panel : String -> List (Html msg) -> String -> Html msg
panel title records emptyMessage =
    section [ class "panel" ]
        [ header title
        , if List.isEmpty records then
            p [ class "panel-empty" ] [ text emptyMessage ]

          else
            div [ class "record-list" ] records
        ]


header : String -> Html msg
header title =
    div [ class "panel-heading" ] [ h2 [] [ text title ] ]


linkedRow : Icon.Icon -> String -> String -> String -> String -> Html msg
linkedRow icon destination title caption outcome =
    a [ href destination, class "record-row" ]
        [ Icon.view icon
        , div [ class "record-copy" ] [ span [ class "strong-text" ] [ text title ], small [] [ text caption ] ]
        , if outcome == "" then
            text ""

          else
            span [ class "trace-count" ] [ text outcome ]
        , span [ class "row-arrow" ] [ Icon.view Icon.Chevron ]
        ]


traceRow : (Scope -> String) -> Scope -> D.Value -> Html msg
traceRow linkTo active row =
    let
        scope =
            referenceScope row
    in
    linkedRow Icon.Branch (linkTo { scope | agent = active.agent }) (fallback (Wire.str "conversation_id" row) scope.trace) (metadata row [ "application_id", "environment", "agent_id", "occurred_at" ]) ""


usageRow : (Scope -> String) -> Scope -> D.Value -> Html msg
usageRow linkTo scope row =
    let
        name =
            Wire.str "name" row

        version =
            Wire.str "version" row

        scopeForRow =
            { scope
                | application = Wire.str "application_id" row
                , environment = Wire.str "environment" row
                , agent = Wire.str "agent_id" row
                , name = name
                , version =
                    if version == "" then
                        Unversioned

                    else
                        ExactVersion version
            }
    in
    linkedRow
        (if scope.mode == "skill" then
            Icon.Book

         else
            Icon.Tools
        )
        (linkTo scopeForRow)
        name
        (metadata row [ "application_id", "environment", "agent_id", "version" ])
        (count "calls" row ++ " calls · " ++ count "succeeded" row ++ " succeeded · " ++ count "failed" row ++ " failed · " ++ count "incomplete" row ++ " incomplete")


callRow : (Scope -> String) -> Scope -> D.Value -> Html msg
callRow linkTo active row =
    let
        scope =
            referenceScope row

        target =
            { active | application = scope.application, environment = scope.environment, trace = scope.trace, event = fallback (Wire.str "finished_event_id" row) (Wire.str "started_event_id" row) }
    in
    linkedRow Icon.Branch
        (linkTo target)
        (fallback (Wire.str "invocation_id" row) scope.trace)
        (String.join " · " (List.filter ((/=) "") [ metadata row [ "agent_id", "version", "occurred_at" ], duration row ]))
        (outcomeText (Wire.str "outcome" row))


metadata : D.Value -> List String -> String
metadata row keys =
    List.map (\key -> Wire.str key row) keys |> List.filter ((/=) "") |> String.join " · "


fallback : String -> String -> String
fallback value_ alternative =
    if value_ == "" then
        alternative

    else
        value_


count : String -> D.Value -> String
count key row =
    D.decodeValue (D.field key D.int) row |> Result.map String.fromInt |> Result.withDefault "—"


outcomeText : String -> String
outcomeText raw =
    case raw of
        "" ->
            "Incomplete"

        "unknown" ->
            "Unknown outcome"

        _ ->
            String.toUpper (String.left 1 raw) ++ (String.dropLeft 1 raw |> String.replace "_" " ")


detail : (Scope -> String) -> Scope -> { allow : Bool, shown : Bool, toggle : msg } -> List D.Value -> Html msg
detail linkTo scope content events =
    let
        traceScope =
            { scope | event = "" }

        listScope =
            { traceScope | trace = "" }

        description =
            String.join " · " [ scope.application, scope.environment ]
    in
    section [ class "panel", attribute "aria-label" "Application trace" ]
        [ div [ class "panel-heading trace-heading" ]
            [ div []
                [ h2 []
                    [ text
                        (if scope.event == "" then
                            "Conversation"

                         else
                            "Linked event"
                        )
                    ]
                , p [] [ text description, text " · ", code [] [ text scope.trace ] ]
                ]
            , div [ class "trace-actions" ]
                [ a [ href (linkTo listScope), class "trace-link" ]
                    [ text
                        (if scope.name == "" then
                            "Back to traces"

                         else
                            "Back to calls"
                        )
                    ]
                , if scope.event == "" then
                    text ""

                  else
                    a [ href (linkTo traceScope), class "trace-link" ] [ text "View full trace" ]
                , if content.allow && (content.shown || List.any hasPayload events) then
                    button [ onClick content.toggle ]
                        [ text
                            (if content.shown then
                                "Hide captured content"

                             else
                                "Show captured content"
                            )
                        ]

                  else
                    text ""
                ]
            ]
        , if List.isEmpty events then
            p [ class "panel-empty" ]
                [ text
                    (if scope.event == "" then
                        "No retained events match this trace."

                     else
                        "The linked event is unavailable. Its trace may still be uploading."
                    )
                ]

          else
            TraceFlow.view scope.event (eventRow content.shown) events
        ]


eventRow : Bool -> D.Value -> Html msg
eventRow showPayload event =
    let
        kind =
            Wire.str "kind" event |> String.replace "_" " " |> outcomeText

        name =
            Wire.str "name" event

        title =
            if name == "" then
                kind

            else
                kind ++ " · " ++ name

        outcome =
            Wire.str "outcome" event

        evidence =
            [ Wire.str "agent_id" event
            , Wire.str "version" event
            , if outcome == "" then
                ""

              else
                outcomeText outcome
            , duration event
            ]

        payload =
            Wire.field "payload" event

        targetLink field caption =
            let
                target =
                    Wire.str field event
            in
            if target == "" then
                ""

            else
                caption ++ " " ++ target
    in
    div [ class "report", attribute "id" ("trace-event-" ++ Wire.str "event_id" event) ]
        [ div [ class "trace-event-heading" ] [ h3 [] [ text title ], small [] [ text (Wire.str "occurred_at" event) ] ]
        , p [ class "trace-event-meta" ] [ text (String.join " · " (List.filter ((/=) "") evidence)) ]
        , small [ class "trace-event-meta" ] [ text (String.join " · " (List.filter ((/=) "") [ targetLink "event_id" "Event", targetLink "turn_id" "Turn", targetLink "invocation_id" "Invocation", targetLink "parent_event_id" "Parent" ])) ]
        , if showPayload then
            payloadView event payload

          else
            payloadStatus event
        ]


duration : D.Value -> String
duration row =
    D.decodeValue (D.field "duration_ms" D.int) row |> Result.map (String.fromInt >> (\value_ -> value_ ++ " ms")) |> Result.withDefault ""


hasPayload : D.Value -> Bool
hasPayload event =
    D.decodeValue (D.field "has_payload" D.bool) event == Ok True


payloadStatus : D.Value -> Html msg
payloadStatus event =
    case Wire.str "payload_state" event of
        "expired" ->
            p [ class "muted" ] [ text "Captured content expired." ]

        "not_captured" ->
            p [ class "muted" ] [ text "Content was not captured." ]

        _ ->
            text ""


payloadView : D.Value -> D.Value -> Html msg
payloadView event payload =
    case D.decodeValue (D.nullable D.value) payload of
        Ok (Just body) ->
            pre [ class "document-text trace-payload" ] [ text (D.decodeValue D.string body |> Result.withDefault (E.encode 2 body)) ]

        _ ->
            payloadStatus event
