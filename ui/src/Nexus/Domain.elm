-- @feature observability-ui
-- @spec docs/features/observability-ui.md
-- @entrypoint dashboardDecoder
-- @boundary dynamic-event-payload
module Nexus.Domain exposing
    ( Bucket
    , Claim
    , Conflict
    , Counts
    , Dashboard
    , Event
    , EventClass(..)
    , ProjectScope(..)
    , ProjectSummary
    , RecordWindow
    , Series
    , Session
    , bucketEvents
    , dashboardDecoder
    , eventMatchesScope
    , projectsDecoder
    )

import Dict exposing (Dict)
import Iso8601
import Json.Decode as Decode exposing (Decoder, Value)
import Json.Decode.Pipeline exposing (optional, required)
import Time exposing (Posix)


type ProjectScope
    = AllProjects
    | OneProject String


type EventClass
    = Activity
    | Info
    | Warning
    | Critical


type alias Counts =
    { activeSessions : Int
    , activeClaims : Int
    , openConflicts : Int
    , events : Int
    }


type alias RecordWindow item =
    { items : List item
    , truncated : Bool
    }


type alias Dashboard =
    { projectId : Maybe String
    , generatedAt : Posix
    , refreshIntervalMs : Int
    , counts : Counts
    , events : RecordWindow Event
    , sessions : RecordWindow Session
    , claims : RecordWindow Claim
    , conflicts : RecordWindow Conflict
    }


type alias Event =
    { id : Int
    , projectId : String
    , sessionId : Maybe String
    , kind : String
    , payload : Value
    , createdAt : Posix
    }


type alias Session =
    { sessionId : String
    , projectId : String
    , agent : String
    , worktree : Maybe String
    , status : String
    , taskSummary : Maybe String
    , lastSeenAt : Posix
    }


type alias Claim =
    { id : String
    , projectId : String
    , sessionId : String
    , toolUseId : String
    , path : String
    , operation : String
    , lineStart : Maybe Int
    , lineEnd : Maybe Int
    , state : String
    , expiresAt : Posix
    , updatedAt : Posix
    }


type alias Conflict =
    { id : String
    , projectId : String
    , leftClaimId : String
    , rightClaimId : String
    , path : String
    , severity : String
    , kind : String
    , status : String
    , message : String
    , createdAt : Posix
    , updatedAt : Posix
    }


type alias ProjectSummary =
    { projectId : String
    , worktrees : List String
    , agents : List String
    , lastSeenAt : Posix
    }


type alias Bucket =
    { start : Posix
    , series : List Series
    }


type alias Series =
    { class : EventClass
    , count : Int
    }


type alias BucketCounts =
    { activity : Int
    , info : Int
    , warning : Int
    , critical : Int
    }


dashboardDecoder : Decoder Dashboard
dashboardDecoder =
    Decode.succeed Dashboard
        |> required "project_id" (Decode.nullable Decode.string)
        |> required "generated_at" Iso8601.decoder
        |> required "refresh_interval_ms" Decode.int
        |> required "counts" countsDecoder
        |> required "events" (windowDecoder eventDecoder)
        |> required "sessions" (windowDecoder sessionDecoder)
        |> required "claims" (windowDecoder claimDecoder)
        |> required "conflicts" (windowDecoder conflictDecoder)


projectsDecoder : Decoder (List ProjectSummary)
projectsDecoder =
    Decode.field "projects" (Decode.list projectDecoder)


countsDecoder : Decoder Counts
countsDecoder =
    Decode.succeed Counts
        |> required "active_sessions" Decode.int
        |> required "active_claims" Decode.int
        |> required "open_conflicts" Decode.int
        |> required "events" Decode.int


windowDecoder : Decoder item -> Decoder (RecordWindow item)
windowDecoder itemDecoder =
    Decode.succeed RecordWindow
        |> required "items" (Decode.list itemDecoder)
        |> required "truncated" Decode.bool


eventDecoder : Decoder Event
eventDecoder =
    Decode.succeed Event
        |> required "id" Decode.int
        |> required "project_id" Decode.string
        |> required "session_id" (Decode.nullable Decode.string)
        |> required "kind" Decode.string
        |> required "payload" Decode.value
        |> required "created_at" Iso8601.decoder


sessionDecoder : Decoder Session
sessionDecoder =
    Decode.succeed Session
        |> required "session_id" Decode.string
        |> required "project_id" Decode.string
        |> required "agent" Decode.string
        |> required "worktree" (Decode.nullable Decode.string)
        |> required "status" Decode.string
        |> required "task_summary" (Decode.nullable Decode.string)
        |> required "last_seen_at" Iso8601.decoder


claimDecoder : Decoder Claim
claimDecoder =
    Decode.succeed Claim
        |> required "id" Decode.string
        |> required "project_id" Decode.string
        |> required "session_id" Decode.string
        |> required "tool_use_id" Decode.string
        |> required "path" Decode.string
        |> required "operation" Decode.string
        |> optional "line_start" (Decode.nullable Decode.int) Nothing
        |> optional "line_end" (Decode.nullable Decode.int) Nothing
        |> required "state" Decode.string
        |> required "expires_at" Iso8601.decoder
        |> required "updated_at" Iso8601.decoder


conflictDecoder : Decoder Conflict
conflictDecoder =
    Decode.succeed Conflict
        |> required "id" Decode.string
        |> required "project_id" Decode.string
        |> required "left_claim_id" Decode.string
        |> required "right_claim_id" Decode.string
        |> required "path" Decode.string
        |> required "severity" Decode.string
        |> required "kind" Decode.string
        |> required "status" Decode.string
        |> required "message" Decode.string
        |> required "created_at" Iso8601.decoder
        |> required "updated_at" Iso8601.decoder


projectDecoder : Decoder ProjectSummary
projectDecoder =
    Decode.succeed ProjectSummary
        |> required "project_id" Decode.string
        |> required "worktrees" (Decode.list Decode.string)
        |> required "agents" (Decode.list Decode.string)
        |> required "last_seen_at" Iso8601.decoder


eventMatchesScope : ProjectScope -> Event -> Bool
eventMatchesScope scope event =
    case scope of
        AllProjects ->
            True

        OneProject projectId ->
            event.projectId == projectId


bucketEvents : Posix -> List Event -> List Bucket
bucketEvents now events =
    let
        currentBucket =
            Time.posixToMillis now // 300000 * 300000

        starts =
            List.range 0 11
                |> List.map (\index -> currentBucket - (11 - index) * 300000)

        initialBuckets =
            List.foldl (\start buckets -> Dict.insert start emptyCounts buckets) Dict.empty starts

        cutoff =
            currentBucket - 11 * 300000

        add event buckets =
            let
                milliseconds =
                    Time.posixToMillis event.createdAt

                bucketStart =
                    milliseconds // 300000 * 300000
            in
            if milliseconds < cutoff then
                buckets

            else
                Dict.update bucketStart
                    (\current -> Just (increment (classify event) (Maybe.withDefault emptyCounts current)))
                    buckets
    in
    events
        |> List.foldl add initialBuckets
        |> Dict.toList
        |> List.map (\( start, counts ) -> Bucket (Time.millisToPosix start) (series counts))


emptyCounts : BucketCounts
emptyCounts =
    BucketCounts 0 0 0 0


increment : EventClass -> BucketCounts -> BucketCounts
increment class counts =
    case class of
        Activity ->
            { counts | activity = counts.activity + 1 }

        Info ->
            { counts | info = counts.info + 1 }

        Warning ->
            { counts | warning = counts.warning + 1 }

        Critical ->
            { counts | critical = counts.critical + 1 }


classify : Event -> EventClass
classify event =
    if event.kind == "conflict_detected" then
        Decode.decodeValue (Decode.field "severity" Decode.string) event.payload
            |> Result.map severityClass
            |> Result.withDefault Info

    else if event.kind == "conflict_reopened" then
        Decode.decodeValue (Decode.field "new_severity" Decode.string) event.payload
            |> Result.map severityClass
            |> Result.withDefault Info

    else
        Activity


severityClass : String -> EventClass
severityClass severity =
    case severity of
        "critical" ->
            Critical

        "warning" ->
            Warning

        _ ->
            Info


series : BucketCounts -> List Series
series counts =
    [ Series Activity counts.activity
    , Series Info counts.info
    , Series Warning counts.warning
    , Series Critical counts.critical
    ]
        |> List.filter (\item -> item.count > 0)
