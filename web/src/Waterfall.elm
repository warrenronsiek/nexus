-- @feature observability-ui
-- @feature work-history
-- @spec docs/features/observability-ui.md
-- @spec docs/features/work-history.md
module Waterfall exposing (Node, build, summary)

{-| Presents a recorded session as one human-rooted tree.

Host capture records a session, its agent container, each human request and its
host turn as separate container spans. They describe the same prompt, so the
view folds agent and turn containers into their parent and names requests as
prompts only when a session has more than one. Consecutive repeated leaf calls
under one parent (for example many `Bash` calls) become a single counted node.
Source records are unchanged; this is presentation only.

-}

import Dict exposing (Dict)
import Json.Decode as D
import Json.Encode as E


type alias Node =
    { id : String, parent : Maybe String, label : String, kind : String, start : Maybe String, end : Maybe String, queued : Maybe String, count : Int, body : D.Value }


folded : Node -> Bool
folded node =
    node.kind == "host.session" || node.kind == "host.turn"


build : Node -> List Node -> List Node
build root intervals =
    root :: (intervals |> foldContainers root.id |> groupRepeats)


foldContainers : String -> List Node -> List Node
foldContainers rootId intervals =
    let
        requests =
            List.filter (\node -> node.kind == "request") intervals |> List.sortBy (\node -> Maybe.withDefault "" node.start)

        dropRequests =
            List.length requests <= 1

        removed =
            List.filter (\node -> folded node || (dropRequests && node.kind == "request")) intervals
                |> List.map (\node -> ( node.id, node.parent ))
                |> Dict.fromList

        resolve depth parent =
            case parent of
                Just id ->
                    if depth > 64 then
                        Just id

                    else
                        case Dict.get id removed of
                            Just next -> resolve (depth + 1) next
                            Nothing -> Just id

                Nothing ->
                    Nothing

        promptNumber =
            requests |> List.indexedMap (\index node -> ( node.id, index + 1 )) |> Dict.fromList

        relabel node =
            case Dict.get node.id promptNumber of
                Just number -> if dropRequests then node else { node | label = "Prompt " ++ String.fromInt number }
                Nothing -> node
    in
    intervals
        |> List.filter (\node -> not (Dict.member node.id removed))
        |> List.map (\node -> relabel { node | parent = resolve 0 node.parent |> Maybe.andThen (\id -> if id == rootId then Nothing else Just id) })


groupRepeats : List Node -> List Node
groupRepeats nodes =
    let
        parents =
            List.filterMap .parent nodes |> List.map (\id -> ( id, () )) |> Dict.fromList

        isLeaf node =
            not (Dict.member node.id parents)

        siblings =
            List.foldr (\node acc -> Dict.update (Maybe.withDefault "" node.parent) (\existing -> Just (node :: Maybe.withDefault [] existing)) acc) Dict.empty nodes
    in
    Dict.values siblings
        |> List.concatMap (\group -> List.sortBy (\node -> Maybe.withDefault "" (orElse node.start node.queued)) group |> runs isLeaf)


runs : (Node -> Bool) -> List Node -> List Node
runs isLeaf ordered =
    let
        step node ( done, current ) =
            case current of
                first :: _ ->
                    if isLeaf node && isLeaf first && first.label == node.label && first.kind == node.kind then
                        ( done, node :: current )

                    else
                        ( merge (List.reverse current) :: done, [ node ] )

                [] ->
                    ( done, [ node ] )

        ( finished, last ) =
            List.foldl step ( [], [] ) ordered
    in
    List.reverse (if List.isEmpty last then finished else merge (List.reverse last) :: finished)


merge : List Node -> Node
merge members =
    case members of
        [ single ] ->
            single

        first :: _ ->
            let
                ends = List.filterMap .end members
                starts = List.filterMap .start members
                allFinished = List.length ends == List.length members
            in
            { first
                | id = "group:" ++ first.id
                , label = first.label ++ " ×" ++ String.fromInt (List.length members)
                , start = List.minimum starts
                , end = if allFinished then List.maximum ends else Nothing
                , queued = Nothing
                , count = List.length members
                , body = E.object [ ( "members", E.list .body members ) ]
            }

        [] ->
            { id = "", parent = Nothing, label = "", kind = "", start = Nothing, end = Nothing, queued = Nothing, count = 0, body = E.null }


orElse : Maybe a -> Maybe a -> Maybe a
orElse fallback preferred =
    case preferred of
        Just _ -> preferred
        Nothing -> fallback


{-| A one-line description of a selected node: kind, recorded elapsed time and outcome. -}
summary : Node -> String
summary node =
    let
        members = D.decodeValue (D.field "members" (D.list D.value)) node.body |> Result.withDefault [ node.body ]
        seconds = List.filterMap elapsedSeconds members
        outcomes = List.map (\body -> D.decodeValue (D.field "outcome" D.string) body |> Result.withDefault "unknown") members
        succeeded = List.length (List.filter ((==) "succeeded") outcomes)
        elapsed = if List.isEmpty seconds then "elapsed unknown" else duration (List.sum seconds) ++ (if List.length seconds < List.length members then " recorded" else "")
        outcome = if node.count > 1 then String.fromInt succeeded ++ "/" ++ String.fromInt node.count ++ " succeeded" else List.head outcomes |> Maybe.withDefault "unknown"
    in
    String.join " · " [ node.label, node.kind, elapsed, outcome ]


elapsedSeconds : D.Value -> Maybe Float
elapsedSeconds body =
    D.decodeValue (D.at [ "elapsed", "nanoseconds" ] D.string) body |> Result.toMaybe |> Maybe.andThen String.toFloat |> Maybe.map (\ns -> ns / 1.0e9)


duration : Float -> String
duration seconds =
    if seconds >= 3600 then String.fromFloat (toFloat (round (seconds / 36)) / 100) ++ "h" else if seconds >= 60 then String.fromFloat (toFloat (round (seconds / 6)) / 10) ++ "m" else String.fromFloat (toFloat (round (seconds * 100)) / 100) ++ "s"
