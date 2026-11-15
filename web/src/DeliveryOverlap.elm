-- @feature delivery-coordination
-- @feature coordination
-- @feature observability-ui
-- @spec docs/features/delivery-coordination.md
-- @spec docs/features/coordination.md
-- @spec docs/features/observability-ui.md
-- @entrypoint view
-- @boundary current-coordination: decode the bounded current snapshot separately from immutable delivery provenance.
module DeliveryOverlap exposing (view)

import Dict exposing (Dict)
import Delivery
import Html exposing (Html, a, code, div, h2, h3, p, section, small, span, text)
import Html.Attributes exposing (attribute, class, href)
import Json.Decode as D exposing (Decoder, Value)
import Set
import State
import Url
import Wire


type alias Claim =
    { id : String, path : String, session : String, operation : String, start : Maybe Int, end : Maybe Int }


type alias Conflict =
    { path : String, severity : String, left : String, right : String }


type alias Snapshot =
    { at : String, claims : List Claim, conflicts : List Conflict, sessions : Dict String String, truncated : Bool }


type alias Contacts =
    { project : Maybe String, claims : Dict String Claim, sessions : Dict String String }


view : Maybe String -> Value -> State.Remote Wire.Document -> Html msg
view project delivery source =
    let
        kind = Wire.valueAt [ "record", "subject", "kind" ] delivery |> D.decodeValue D.string |> Result.withDefault ""
        files = contributionFiles delivery
    in
    if not (List.member kind [ "pull_request", "contribution" ]) then
        text ""

    else
        section [ class "panel delivery-overlap", attribute "aria-label" "Current delivery overlaps" ]
            [ div [ class "panel-heading" ] [ div [] [ h2 [] [ text "Current claims and overlaps" ], p [] [ text "Historical contribution mappings identify these files. Current claims are advisory activity signals and do not establish authorship." ] ] ]
            , source.error |> Maybe.map (\reason -> div [ class "notice error" ] [ text ("Current overlap refresh failed. " ++ reason ++ (if source.value == Nothing then "" else " Showing the last successful coordination snapshot.")) ]) |> Maybe.withDefault (text "")
            , if source.loading then p [ class "panel-empty" ] [ text "Refreshing current coordination snapshot…" ] else text ""
            , case source.value of
                Nothing -> p [ class "panel-empty" ] [ text "Current overlap coverage is unavailable until a coordination snapshot is read." ]
                Just document ->
                    case D.decodeValue snapshot document.root of
                        Err _ -> div [ class "notice error" ] [ text "The current coordination snapshot is incomplete or invalid. Overlap coverage is unknown." ]
                        Ok current -> snapshotView project files current
            ]


snapshotView : Maybe String -> Dict String (List String) -> Snapshot -> Html msg
snapshotView project files current =
    let
        contacts = Contacts project (Dict.fromList (List.map (\claim -> ( claim.id, claim )) current.claims)) current.sessions
    in
    div []
        [ p [ class "delivery-overlap-source" ]
            [ text "Coordination snapshot: ", Html.time [ attribute "datetime" current.at, attribute "title" current.at ] [ text current.at ]
            , text (" · " ++ String.fromInt (List.length current.claims) ++ " sampled claims · " ++ String.fromInt (List.length current.conflicts) ++ " sampled open overlaps")
            ]
        , if current.truncated then p [ class "notice" ] [ text "Coordination sample is truncated; a missing match does not establish absence outside the observed window." ] else text ""
        , if Dict.isEmpty files then p [ class "panel-empty" ] [ text "No mapped contribution files are retained for this record. Overlap coverage cannot be assessed." ] else div [] (List.map (fileView contacts current) (Dict.toList files))
        ]


fileView : Contacts -> Snapshot -> ( String, List String ) -> Html msg
fileView contacts current ( path, ranges ) =
    let
        claims = List.filter (\claim -> claim.path == path) current.claims
        conflicts = List.filter (\conflict -> conflict.path == path) current.conflicts
    in
    div [ class "delivery-overlap-file" ]
        [ h3 [] [ code [] [ text path ] ]
        , small [ class "delivery-mapped-ranges" ] [ text ("Recorded mapping ranges: " ++ String.join "; " (Set.toList (Set.fromList ranges))) ]
        , if List.isEmpty claims then p [ class "muted" ] [ text "No current claims on this path in the observed window." ] else div [ class "delivery-current-claims" ] (List.map (claimView contacts) claims)
        , if List.isEmpty conflicts then p [ class "muted" ] [ text "No overlap in the observed coordination window." ] else div [ class "delivery-current-conflicts" ] (List.map (conflictView contacts) conflicts)
        ]


claimView : Contacts -> Claim -> Html msg
claimView contacts claim =
    p [] [ span [ class "delivery-overlap-label" ] [ text "Current claim" ], sessionLink contacts claim.session, text (" · " ++ claim.operation ++ " · " ++ claimRange claim) ]


claimRange : Claim -> String
claimRange claim =
    case claim.start of
        Nothing -> "whole path"
        Just start -> "lines " ++ String.fromInt start ++ "–" ++ String.fromInt (Maybe.withDefault start claim.end)


conflictView : Contacts -> Conflict -> Html msg
conflictView contacts conflict =
    p [] [ span [ class "delivery-overlap-label" ] [ text (conflict.severity ++ " overlap") ], counterpart contacts conflict.left, text " ↔ ", counterpart contacts conflict.right ]


counterpart : Contacts -> String -> Html msg
counterpart contacts claimId =
    case Dict.get claimId contacts.claims of
        Just claim -> sessionLink contacts claim.session
        Nothing -> span [] [ text "Session unavailable in sample", code [] [ text (" (claim " ++ claimId ++ ")") ] ]


sessionLink : Contacts -> String -> Html msg
sessionLink contacts identity =
    let
        project = contacts.project |> Maybe.map (\id -> "&project=" ++ Url.percentEncode id) |> Maybe.withDefault ""
        name = Dict.get identity contacts.sessions |> Maybe.withDefault identity
    in
    a [ href ("/ui/coordination?session=" ++ Url.percentEncode identity ++ project) ] [ text name ]


contributionFiles : Value -> Dict String (List String)
contributionFiles delivery =
    (Wire.field "record" delivery :: Wire.list "related" delivery)
        |> List.concatMap (Wire.field "body" >> Wire.list "mappings")
        |> List.concatMap (Wire.list "hunks")
        |> List.filter (\hunk -> Wire.str "path" hunk /= "")
        |> List.foldl (\hunk -> Dict.update (Wire.str "path" hunk) (\known -> Just (mappedRange hunk :: Maybe.withDefault [] known))) Dict.empty


mappedRange : Value -> String
mappedRange hunk =
    "old " ++ Delivery.range "old" hunk ++ " → new " ++ Delivery.range "new" hunk


snapshot : Decoder Snapshot
snapshot =
    D.map5 Snapshot
        (D.field "generated_at" D.string)
        (D.at [ "claims", "items" ] (D.list claimDecoder))
        (D.at [ "conflicts", "items" ] (D.list conflictDecoder))
        (D.at [ "sessions", "items" ] (D.list (D.map2 (\identity body -> ( identity, Wire.sessionName body )) (D.field "session_id" D.string) D.value)) |> D.map Dict.fromList)
        (D.map3 (\claims conflicts sessions -> claims || conflicts || sessions) (D.at [ "claims", "truncated" ] D.bool) (D.at [ "conflicts", "truncated" ] D.bool) (D.at [ "sessions", "truncated" ] D.bool))


claimDecoder : Decoder Claim
claimDecoder =
    D.map6 Claim (D.field "id" D.string) (D.field "path" D.string) (D.field "session_id" D.string) (D.field "operation" D.string) (Wire.optional "line_start" (D.nullable D.int) Nothing) (Wire.optional "line_end" (D.nullable D.int) Nothing)


conflictDecoder : Decoder Conflict
conflictDecoder =
    D.map4 Conflict (D.field "path" D.string) (D.field "severity" D.string) (D.field "left_claim_id" D.string) (D.field "right_claim_id" D.string)
