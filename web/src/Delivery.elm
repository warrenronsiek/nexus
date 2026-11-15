-- @feature delivery-coordination
-- @feature observability-ui
-- @spec docs/features/delivery-coordination.md
-- @spec docs/features/observability-ui.md
-- @entrypoint view
-- @boundary delivery-json: display the closed delivery response and its typed record variants; unknown observations stay visibly unknown.
module Delivery exposing (range, view)

import Html exposing (Html, a, code, dd, details, div, dl, dt, h2, h3, p, section, small, span, summary, text)
import Html.Attributes exposing (attribute, class, href)
import Json.Decode as D exposing (Value)
import Url
import Wire


view : Maybe String -> String -> Value -> Maybe Value -> Html msg
view project home root watches =
    div [ class "delivery" ]
        [ case Wire.str "response" root of
            "detail" -> recordDetail project home root
            _ -> recordList project root
        , watchView root watches
        ]


recordList : Maybe String -> Value -> Html msg
recordList project root =
    let
        records = Wire.list "items" root
    in
    panel "Delivery registry" "Recorded responsibility remains visible when an agent or checkout is no longer active."
        [ if List.isEmpty records then
            p [ class "panel-empty" ] [ text "No delivery records in this scope. Agents register contributions and PR responsibility through nexus_delivery or nexus delivery." ]
          else
            div [ class "delivery-list" ] (List.map (recordRow project) records)
        ]


recordRow : Maybe String -> Value -> Html msg
recordRow project record =
    let
        body = Wire.field "body" record
        pr = Wire.field "pull_request" body
        contribution = Wire.field "contribution" body
        remediation = Wire.field "remediation" body
        origin = Wire.field "origin" contribution
        person = if Wire.str "kind" body == "contribution" then agentName origin else if Wire.str "kind" body == "remediation" then agentName (Wire.field "recipient" remediation) else shepherdName record
        snapshot = Wire.field "snapshot" body
        commit = revision (Wire.field "head" snapshot)
        recordedHead = revision (Wire.field "head" pr)
    in
    a [ class "delivery-row", href (recordUrl project (Wire.field "subject" record)) ]
        [ div [ class "delivery-name" ] [ span [ class "delivery-kind" ] [ text (human (Wire.str "kind" body)) ], Html.strong [] [ text (Wire.str "title" record) ], small [] [ text (Wire.str "repository" record) ] ]
        , div [] [ small [] [ text (if Wire.str "kind" body == "remediation" then "Requested agent" else if Wire.str "kind" body == "contribution" then "Original agent" else "Shepherd") ], span [] [ text person ], small [] [ text ("Availability: " ++ human (Wire.str "reachability" (Wire.field "shepherd_presence" record))) ] ]
        , div [] [ small [] [ text "Recorded state" ], span [ class (stateClass (Wire.str "state" record)) ] [ text (human (Wire.str "state" record)) ], if Wire.str "kind" body == "pull_request" then small [] [ text (reference "head_ref" snapshot pr ++ " → " ++ reference "base_ref" snapshot pr) ] else text "", if commit /= "Unknown" || recordedHead /= "Unknown" then code [ class "delivery-hash" ] [ text (if commit == "Unknown" then recordedHead else commit) ] else text "" ]
        , div [] [ small [] [ text "Last record" ], sourceTime (Wire.str "updated_at" record), span [ class "delivery-open" ] [ text "Inspect →" ] ]
        ]


recordDetail : Maybe String -> String -> Value -> Html msg
recordDetail project home root =
    let
        record = Wire.field "record" root
        related = Wire.list "related" root
        observations = Wire.list "observations" root
        body = Wire.field "body" record
        kind = Wire.str "kind" body
    in
    if Wire.str "title" record == "" then
        panel "Record unavailable" "" [ p [ class "panel-empty" ] [ text "This record was not found in the selected project." ], a [ href home, class "delivery-back" ] [ text "← All pull requests" ] ]

    else
        div []
            [ a [ href home, class "delivery-back" ] [ text "← All pull requests" ]
            , div [ class "delivery-detail-heading" ]
                [ div [] [ small [ class "delivery-kind" ] [ text (human kind) ], h2 [] [ text (Wire.str "title" record) ], p [] [ text (Wire.str "repository" record) ] ]
                , span [ class (stateClass (Wire.str "state" record)) ] [ text (human (Wire.str "state" record)) ]
                ]
            , if D.decodeValue (D.field "truncated" D.bool) root == Ok True then div [ class "notice" ] [ text "Related records or observation history are partial. Use the CLI event and export cursors for complete retained evidence." ] else text ""
            , div [ class "delivery-detail-grid" ]
                [ div [] [ bodyView project record, relatedPanel project "Original contributions" "contribution" related, relatedPanel project "Deployment attempts" "deployment" related ]
                , div [] [ if List.member kind [ "pull_request", "rollout" ] then responsibility record else text "", relatedPanel project "Rollout" "rollout" related, relatedPanel project "Remediation requests" "remediation" related, relatedPanel project "Pull requests" "pull_request" related ]
                ]
            , history observations
            ]


bodyView : Maybe String -> Value -> Html msg
bodyView project record =
    let
        body = Wire.field "body" record
    in
    case Wire.str "kind" body of
        "pull_request" -> pullRequest body
        "contribution" -> contributionView project body
        "rollout" ->
            let rollout = Wire.field "rollout" body in
            panel "Rollout scope" "Promotion and deployment remain caller-owned."
                [ facts [ ( "Environments", String.join " → " (Wire.strings "environments" rollout) ), ( "Source PRs", String.join ", " (Wire.strings "pull_request_ids" rollout) ), ( "Contributions", String.join ", " (Wire.strings "contribution_ids" rollout) ) ] ]
        "deployment" -> deploymentView (Wire.field "deployment" body)
        "remediation" -> remediationView project body
        _ -> panel "Delivery evidence" "" [ p [ class "panel-empty" ] [ text "This record has no supported evidence view." ] ]


pullRequest : Value -> Html msg
pullRequest body =
    let
        pr = Wire.field "pull_request" body
        snapshot = Wire.field "snapshot" body
        observed = Wire.str "observed_at" snapshot
        checks = Wire.list "checks" snapshot
    in
    panel "Revision and checks" "Checks describe their recorded revision. An empty list does not establish passing checks."
        [ div [ class "delivery-section" ] [ external "Open pull request ↗" (Wire.str "url" pr) ]
        , facts [ ( "Provider PR", Wire.str "provider" pr ++ " #" ++ String.fromFloat (Wire.number "number" pr) ), ( "Source → target", reference "head_ref" snapshot pr ++ " → " ++ reference "base_ref" snapshot pr ), ( "Registered base", revision (Wire.field "base" pr) ), ( "Registered head", revision (Wire.field "head" pr) ), ( "Observed head", revision (Wire.field "head" snapshot) ), ( "Snapshot observed at", known observed ) ]
        , if observed == "" then p [ class "panel-empty" ] [ text "No provider snapshot observed." ] else text ""
        , if List.isEmpty checks then p [ class "panel-empty" ] [ text "No check observations recorded." ] else div [ class "delivery-checks" ] (List.map checkView checks)
        ]


checkView : Value -> Html msg
checkView check =
    div [ class "delivery-check" ]
        [ div [] [ external (Wire.str "name" check) (Wire.str "url" check), code [ class "delivery-hash" ] [ text (revision (Wire.field "revision" check)) ] ]
        , span [ class (stateClass (Wire.str "state" check)) ] [ text (human (Wire.str "state" check)) ]
        ]


contributionView : Maybe String -> Value -> Html msg
contributionView project body =
    let
        contribution = Wire.field "contribution" body
        work = Wire.str "work_session_id" contribution
        constraints = Wire.strings "constraints" contribution
    in
    panel "Original implementation" (agentName (Wire.field "origin" contribution))
        [ div [ class "delivery-section prose" ] [ h3 [] [ text "Why this changed" ], p [] [ text (known (Wire.str "rationale" contribution)) ], if List.isEmpty constraints then text "" else p [] [ text ("Constraints: " ++ String.join "; " constraints) ] ]
        , facts [ ( "Features", known (String.join ", " (Wire.strings "features" contribution)) ), ( "Branch", known (Wire.str "branch" contribution) ), ( "Worktree", known (Wire.str "worktree" contribution) ), ( "Base revision", revision (Wire.field "base_revision" contribution) ), ( "Head revision", revision (Wire.field "head_revision" contribution) ) ]
        , if work == "" then text "" else div [ class "delivery-section" ] [ a [ href ("/ui/work?session=" ++ Url.percentEncode work ++ projectQuery project) ] [ text "Open original work →" ] ]
        , routeView (Wire.field "route" contribution)
        , div [ class "delivery-mappings" ] (if List.isEmpty (Wire.list "mappings" body) then [ p [ class "panel-empty" ] [ text "No file or revision mapping recorded. Feature membership alone is candidate evidence." ] ] else List.map mappingView (Wire.list "mappings" body))
        ]


mappingView : Value -> Html msg
mappingView mapping =
    div [ class "delivery-mapping" ]
        [ div [ class "delivery-check" ] [ code [ class "delivery-hash" ] [ text (revision (Wire.field "revision" mapping)) ], span [] [ text (human (Wire.str "verification" mapping) ++ " · " ++ human (Wire.str "operation" mapping)) ] ]
        , div [ class "delivery-hunks" ] (List.map (\hunk -> p [] [ code [] [ text (Wire.str "path" hunk) ], span [] [ text (" · old " ++ range "old" hunk ++ " → new " ++ range "new" hunk) ], if Wire.str "old_path" hunk == "" then text "" else small [] [ text ("Renamed from " ++ Wire.str "old_path" hunk) ] ]) (Wire.list "hunks" mapping))
        ]


range : String -> Value -> String
range side hunk =
    let start = round (Wire.number (side ++ "_start") hunk)
        count = round (Wire.number (side ++ "_lines") hunk)
    in
    if count == 0 then "no lines" else if count == 1 then String.fromInt start else String.fromInt start ++ "–" ++ String.fromInt (start + count - 1)


deploymentView : Value -> Html msg
deploymentView deployment =
    panel "Environment attempt" "Each attempt retains its own revision and observed outcome."
        [ facts [ ( "Environment", Wire.str "environment" deployment ), ( "Attempt", String.fromFloat (Wire.number "attempt" deployment) ), ( "Revision", revision (Wire.field "revision" deployment) ), ( "Artifact", known (Wire.str "artifact_digest" deployment) ), ( "Rollback of", known (Wire.str "rollback_of" deployment) ) ]
        , div [ class "delivery-section" ] [ external "Open deployment evidence ↗" (Wire.str "external_url" deployment) ]
        ]


remediationView : Maybe String -> Value -> Html msg
remediationView project body =
    let request = Wire.field "remediation" body in
    panel "Remediation request" "Recording a request does not prove host delivery or agent acceptance."
        [ div [ class "delivery-section prose" ] [ p [] [ text (Wire.str "request" request) ] ]
        , facts [ ( "Requested agent", agentName (Wire.field "recipient" request) ), ( "Failure revision", revision (Wire.field "failure_revision" request) ), ( "Current reason", known (Wire.str "reason" body) ), ( "Fixing revision", revision (Wire.field "fixing_revision" body) ) ]
        , div [ class "delivery-section prose" ] (List.map (\evidence -> p [] [ text evidence ]) (Wire.strings "evidence" request))
        , linkedId project "contribution" "Original contribution" (Wire.str "contribution_id" request)
        , linkedId project "pull_request" "Pull request" (Wire.str "pull_request_id" request)
        , linkedId project "deployment" "Failed deployment" (Wire.str "deployment_id" request)
        ]


responsibility : Value -> Html msg
responsibility record =
    let
        assignment = Wire.field "shepherd" record
        agent = Wire.field "agent" assignment
        presence = D.decodeValue (D.nullable D.value) (Wire.field "shepherd_presence" record) |> Result.withDefault Nothing
    in
    panel "Shepherd" "Responsibility and observed availability are separate."
        [ facts [ ( "Responsible agent", shepherdName record ), ( "Session", known (Wire.str "session_id" agent) ), ( "Assigned at", known (Wire.str "assigned_at" assignment) ), ( "Reason", known (Wire.str "reason" assignment) ), ( "Availability", presence |> Maybe.map (Wire.str "reachability" >> human) |> Maybe.withDefault "Unknown" ) ]
        , presence |> Maybe.map (\value -> div [] [ facts [ ( "Presence observed at", Wire.str "observed_at" value ) ], routeView (Wire.field "route" value) ]) |> Maybe.withDefault (text "")
        ]


relatedPanel : Maybe String -> String -> String -> List Value -> Html msg
relatedPanel project heading kind related =
    let
        records = List.filter (\record -> Wire.str "kind" (Wire.field "subject" record) == kind) related
        row record =
            a [ href (recordUrl project (Wire.field "subject" record)) ]
                [ Html.strong [] [ text (Wire.str "title" record) ]
                , small [] [ text (human (Wire.str "state" record)) ]
                , if kind == "contribution" then
                    p []
                        [ text (agentName (Wire.valueAt [ "body", "contribution", "origin" ] record))
                        , small [] [ text (Wire.str "rationale" (Wire.valueAt [ "body", "contribution" ] record)) ]
                        , small [] [ text ("Features: " ++ known (String.join ", " (Wire.strings "features" (Wire.valueAt [ "body", "contribution" ] record)))) ]
                        ]
                  else text ""
                ]
    in
    if List.isEmpty records then text "" else panel heading "" [ div [ class "delivery-related" ] (List.map row records) ]


history : List Value -> Html msg
history observations =
    details [ class "panel delivery-history" ]
        [ summary [] [ text "Responsibility and observation history" ]
        , if List.isEmpty observations then p [ class "panel-empty" ] [ text "No observation history retained for this record." ] else div [ class "delivery-history-rows" ] (List.map historyRow observations)
        ]


historyRow : Value -> Html msg
historyRow observation =
    let
        input = Wire.field "input" observation
        event = Wire.field "body" input
    in
    div [] [ sourceTime (Wire.str "occurred_at" input), Html.strong [] [ text (human (Wire.str "event" event)) ], p [] [ text (Wire.str "reason" event) ], small [] [ text ("Recorded " ++ Wire.str "recorded_at" observation) ] ]


watchView : Value -> Maybe Value -> Html msg
watchView source watches =
    panel "Watch evidence" "A retained snapshot is not a live connection to the Git provider."
        [ case watches of
            Nothing -> p [ class "panel-empty" ] [ text "Watch status unavailable. Use Refresh to retrieve retained observations." ]
            Just root ->
                let
                    subject = Wire.valueAt [ "record", "subject" ] source
                    matches watch = Wire.str "kind" subject /= "pull_request" || Wire.str "pull_request_id" (Wire.field "registration" watch) == Wire.str "id" subject
                    items = List.filter matches (Wire.list "watches" root)
                in
                if List.isEmpty items then p [ class "panel-empty" ] [ text "No watcher registered in this scope. Provider freshness is unknown." ] else div [ class "delivery-watch-list" ] (List.map watchFacts items)
        ]


watchFacts : Value -> Html msg
watchFacts watch =
    let
        registration = Wire.field "registration" watch
        enabled = D.decodeValue (D.field "enabled" D.bool) watch |> Result.toMaybe
    in
    facts [ ( "Watch", known (Wire.str "id" registration) ), ( "PR record", Wire.str "pull_request_id" registration ), ( "State", enabled |> Maybe.map (\active -> if active then "Enabled" else "Stopped") |> Maybe.withDefault "Unknown" ), ( "Interval", String.fromFloat (Wire.number "interval_seconds" registration) ++ " seconds" ), ( "Last attempt", known (Wire.str "last_attempt_at" watch) ), ( "Last successful observation", known (Wire.str "last_success_at" watch) ), ( "Next poll due", known (Wire.str "next_poll_at" watch) ), ( "Last failure", known (Wire.str "last_error" watch) ) ]


routeView : Value -> Html msg
routeView route =
    if Wire.str "host" route == "" then text "" else div [ class "delivery-section" ] [ h3 [] [ text "Contact route" ], p [] [ text (Wire.str "host" route ++ " · " ++ Wire.str "thread_id" route) ], external "Open original chat ↗" (Wire.str "url" route) ]


linkedId : Maybe String -> String -> String -> String -> Html msg
linkedId project kind caption identity =
    if identity == "" then text "" else div [ class "delivery-section" ] [ a [ href (subjectUrl project kind identity) ] [ text (caption ++ " →") ] ]


recordUrl : Maybe String -> Value -> String
recordUrl project subject =
    subjectUrl project (Wire.str "kind" subject) (Wire.str "id" subject)


subjectUrl : Maybe String -> String -> String -> String
subjectUrl project kind identity =
    "/ui/pull-requests?record=" ++ Url.percentEncode (kind ++ ":" ++ identity) ++ projectQuery project


projectQuery : Maybe String -> String
projectQuery project =
    project |> Maybe.map (\identity -> "&project=" ++ Url.percentEncode identity) |> Maybe.withDefault ""


shepherdName : Value -> String
shepherdName record =
    let agent = Wire.valueAt [ "shepherd", "agent" ] record in
    if Wire.str "agent" agent == "" then "Unassigned" else agentName agent


agentName : Value -> String
agentName agent =
    if Wire.str "agent" agent == "" then "Unknown" else Wire.str "agent" agent ++ (if Wire.str "session_id" agent == "" then "" else " · " ++ Wire.str "session_id" agent)


revision : Value -> String
revision value =
    known (Wire.str "commit" value)


reference : String -> Value -> Value -> String
reference key snapshot registered =
    let observed = Wire.str key snapshot in
    known (if observed == "" then Wire.str key registered else observed)


known : String -> String
known value =
    if String.trim value == "" then "Unknown" else value


human : String -> String
human value =
    known (String.replace "_" " " value)


stateClass : String -> String
stateClass state =
    "delivery-state " ++ (if List.member state [ "failed", "unreachable", "declined" ] then "failure" else if List.member state [ "succeeded", "resolved", "merged" ] then "success" else "")


sourceTime : String -> Html msg
sourceTime stamp =
    if stamp == "" then span [] [ text "Unknown" ] else Html.time [ attribute "datetime" stamp, attribute "title" stamp ] [ text (String.replace "T" " " stamp) ]


external : String -> String -> Html msg
external caption url =
    if String.startsWith "https://" url || String.startsWith "http://" url then a [ href url ] [ text caption ] else if url == "" then text "" else span [] [ text (caption ++ ": " ++ url) ]


facts : List ( String, String ) -> Html msg
facts entries =
    dl [ class "delivery-facts" ] (List.concatMap (\( caption, value ) -> [ dt [] [ text caption ], dd [] [ text (known value) ] ]) entries)


panel : String -> String -> List (Html msg) -> Html msg
panel heading caption body =
    section [ class "panel" ] (div [ class "panel-heading" ] [ div [] [ h2 [] [ text heading ], if caption == "" then text "" else p [] [ text caption ] ] ] :: body)
