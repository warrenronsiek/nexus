-- @feature evals
-- @feature observability-ui
-- @spec docs/features/evals.md
-- @spec docs/features/observability-ui.md
-- @boundary eval-presentation: exact source values and definition identities remain authoritative; charts never compute a new score.
module Evals exposing (Field, Scope, agents, configurations, empty, metrics, set, view)

import Dict exposing (Dict)
import Html exposing (Html, code, details, div, h2, h3, label, option, p, section, select, small, strong, summary, text)
import Html.Attributes exposing (attribute, class, selected, value)
import Html.Events exposing (onInput)
import Json.Decode as D
import Wire exposing (Document, Metric, Row)


type alias Scope =
    { model : String, definitions : Dict String String }


type Field = Model | Definition String


empty : Scope
empty = Scope "" Dict.empty


set : Field -> String -> Scope -> Scope
set field picked scope =
    case field of
        Model -> { scope | model = picked }
        Definition family -> { scope | definitions = Dict.insert family picked scope.definitions }


agents : List Row -> List Row
agents rows =
    rows |> List.foldl (\row found ->
        let agent = Wire.str "agent" row.body in
        if agent == "" then Dict.insert row.id row found else Dict.insert agent { row | id = agent, title = agent, subtitle = "Recorded configurations", kind = "eval_agent" } found) Dict.empty |> Dict.values



configurations : String -> List Row -> Html msg
configurations agent rows =
    let
        matching = List.filter (\row -> row.id == agent || Wire.str "agent" row.body == agent) rows
        configuration row =
            div [ class "metric-definition" ]
                [ h3 [] [ text row.title ]
                , p [] [ text (String.join " · " (Wire.strings "models" row.body)) ]
                , p [] [ text (String.join " · " (Wire.strings "components" row.body)) ]
                , p [] [ code [] [ text row.id ] ]
                , p [ class "muted" ] [ text ("Suite revisions: " ++ String.join ", " (Wire.strings "suite_revision_ids" row.body)) ]
                , p [ class "muted" ] [ text ("Target revisions: " ++ String.join ", " (Wire.strings "target_revision_ids" row.body)) ]
                ]
    in
    details [ class "eval-configurations" ]
        [ summary [] [ text (String.fromInt (List.length matching) ++ " recorded configurations") ]
        , div [] (List.map configuration matching)
        ]

type alias Family = { key : String, label : String, unit : String, definitions : List Metric }


families : List Metric -> List Family
families source =
    source |> List.foldl (\metric grouped ->
        let key = metric.label ++ "\u{001F}" ++ metric.unit ++ "\u{001F}" ++ metric.scope in
        Dict.update key (\existing -> Just (case existing of
            Just family -> { family | definitions = family.definitions ++ [ metric ] }
            Nothing -> Family key metric.label metric.unit [ metric ])) grouped) Dict.empty |> Dict.values


recipe : Scope -> Family -> List Metric
recipe scope family =
    let
        chosen = Dict.get family.key scope.definitions |> Maybe.andThen (\definition -> List.filter (\metric -> metric.definition == definition) family.definitions |> List.head)
        latest = family.definitions |> List.sortBy (\metric -> metric.points |> List.map .at |> List.maximum |> Maybe.withDefault "") |> List.reverse |> List.head
    in
    chosen |> orElse latest |> Maybe.map (\definition -> List.filter (\metric -> metric.calculation == definition.calculation) family.definitions) |> Maybe.withDefault []


orElse : Maybe a -> Maybe a -> Maybe a
orElse fallback preferred =
    case preferred of
        Just _ -> preferred
        Nothing -> fallback


plot : Scope -> Family -> Maybe Metric
plot scope family =
    let
        chosen = recipe scope family
        points metric = metric.points |> List.filter (\point -> scope.model == "" || List.member scope.model point.models)
            |> List.map (\point -> { point | label = String.join " · " (List.filter ((/=) "") [ String.join ", " point.models, metric.definition, point.label ]) })
    in
    List.head chosen |> Maybe.map (\first -> { first | points = List.concatMap points chosen |> List.sortBy .at })


metrics : Scope -> List Metric -> List Metric
metrics scope source =
    families source |> List.filterMap (plot scope)


view : (String -> Html msg) -> (Field -> String -> msg) -> Scope -> List Metric -> Html msg
view chart change scope source =
    let
        models = source |> List.concatMap .points |> List.concatMap .models |> List.map (\model -> ( model, () )) |> Dict.fromList |> Dict.keys
        picker caption current choices onChange =
            label [ class "scope-control" ] [ small [] [ text caption ], select [ attribute "aria-label" caption, onInput onChange, value current ] (List.map (\( key, title ) -> option [ value key, selected (key == current) ] [ text title ]) choices) ]
        panel index family =
            let
                chosen = recipe scope family
                active = Dict.get family.key scope.definitions |> Maybe.withDefault (List.head chosen |> Maybe.map .definition |> Maybe.withDefault "")
                plotted = plot scope family
                values = plotted |> Maybe.map .points |> Maybe.withDefault []
                definition metric = div [ class "metric-definition" ] [ code [] [ text metric.definition ], p [] [ text metric.description ], calculation metric.calculation ]
            in
            section [ class "panel" ]
                [ div [ class "panel-heading" ] [ div [] [ h2 [] [ text family.label ], p [] [ text family.unit ] ], picker "Definition" active (List.map (\metric -> ( metric.definition, metric.definition )) family.definitions) (change (Definition family.key)) ]
                , chart ("metric-" ++ String.fromInt index)
                , p [ class "muted" ] [ text "Recorded observations; no score is recomputed. Different calculation recipes are selected separately." ]
                , div [ class "result-values" ] (values |> List.reverse |> List.take 6 |> List.map (\point -> div [ class "result-value" ] [ small [] [ text point.at ], strong [] [ text point.exact ] ]))
                , details [] [ summary [] [ text ("Definitions and source observations (" ++ String.fromInt (List.length values) ++ ")") ], div [] (List.map definition chosen), div [] (values |> List.reverse |> List.take 32 |> List.map (\point -> p [] [ code [] [ text point.id ], text (" · " ++ point.label ++ " · " ++ point.exact) ])), if List.length values > 32 then p [] [ text "Showing the latest 32 captured observations." ] else text "" ]
                ]
    in
    div []
        [ if List.isEmpty models then text "" else picker "Model" scope.model (( "", "All recorded models" ) :: List.map (\model -> ( model, model )) models) (change Model)
        , div [ class "metric-grid" ] (List.indexedMap panel (families source))
        ]


calculation : String -> Html msg
calculation source =
    case D.decodeString (D.keyValuePairs D.value) source of
        Ok fields -> div [] (fields |> List.filterMap (\( key, item ) -> D.decodeValue D.string item |> Result.toMaybe |> Maybe.map (\value -> p [] [ strong [] [ text (String.replace "_" " " key ++ ". ") ], text value ])))
        Err _ -> p [] [ text source ]
