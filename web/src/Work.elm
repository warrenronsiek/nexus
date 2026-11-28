-- @feature work-history
-- @feature observability-ui
-- @spec docs/features/work-history.md
-- @spec docs/features/observability-ui.md
-- @boundary work-detail: retained source observations render as inert text, never executable commands.
module Work exposing (detail)

import Html exposing (Html, a, code, dd, div, dl, dt, h3, h4, p, section, text)
import Html.Attributes exposing (attribute, class, href)
import Json.Decode as D
import Waterfall
import Wire exposing (Document)


detail : (String -> String) -> Waterfall.Node -> Document -> Html msg
detail sessionLink node loaded =
    let
        root = loaded.root
        source = Wire.field "detail" root
        summary = Wire.field "summary" source
        context = Wire.field "context" summary
        facts =
            [ ( "Kind", node.kind )
            , ( "State", Wire.str "state" summary )
            , ( "Outcome", Wire.str "outcome" summary )
            , ( "Executor", Wire.str "executor" summary )
            , ( "Repository", Wire.str "repository" context )
            , ( "Worktree", Wire.str "worktree" context )
            , ( "Revision", Wire.str "commit" (Wire.field "revision" context) )
            , ( "Started", Wire.str "started_at" summary )
            , ( "Finished", Wire.str "finished_at" summary )
            , ( "Parent activity", Wire.str "parent_activity_id" summary )
            ]
        operations = Wire.list "operations" summary
        observations = Wire.list "observations" source
        operation item =
            div [ class "work-operation" ]
                [ h4 [] [ code [] [ text (Wire.str "name" item) ] ]
                , p [] [ text (Wire.str "description" item) ]
                , p [ class "muted" ] [ text (String.join " · " [ Wire.str "category_name" item, Wire.str "evidence" item ]) ]
                ]
        observation item =
            let body = Wire.field "body" item in
            div [ class "work-observation" ]
                [ code [] [ text (Wire.str "occurred_at" item) ]
                , p [] [ text (String.join " · " (List.filter ((/=) "") [ Wire.str "event" body, Wire.str "name" body, Wire.str "outcome" body ])) ]
                ]
    in
    section [ class "work-inspection", attribute "aria-label" "Work activity details", attribute "aria-live" "polite" ]
        [ h3 [] [ text node.label ]
        , dl [ class "evidence-facts" ] (facts |> List.filter (Tuple.second >> (/=) "") |> List.concatMap (\( name, content ) -> [ dt [] [ text name ], dd [] [ text content ] ]))
        , div [] (List.map operation operations)
        , if List.isEmpty operations then p [ class "muted" ] [ text "No correlated operation details were retained for this call." ] else text ""
        , p [ class "muted" ] [ text "Raw arguments, environment values and output are not retained by automatic capture." ]
        , div [] (Wire.list "links" source |> List.filterMap (\link ->
            let target = Wire.field "target" link in
            if Wire.str "kind" target == "session" then Just (p [] [ a [ href (sessionLink (Wire.str "id" target)) ] [ text ("Open delegated session · " ++ Wire.str "id" target) ] ]) else Nothing))
        , h4 [] [ text "Recorded observations" ]
        , div [] (List.map observation observations)
        , if D.decodeValue (D.field "truncated" D.bool) source == Ok True then p [ class "notice" ] [ text "Observation history is partial. More evidence is available through the source event reads." ] else text ""
        ]
