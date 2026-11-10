-- @feature code-map
-- @feature observability-ui
-- @spec docs/features/code-map.md
-- @spec docs/features/observability-ui.md
-- @boundary code-map-view: decode project-scoped measurements and opaque grouping keys from the read adapter
module CodeMap exposing (Evidence, chart, filters, view)

import Html exposing (Html, button, div, label, li, option, p, select, span, text, ul)
import Html.Attributes exposing (attribute, class, selected, value)
import Html.Events exposing (onClick, onInput)
import Json.Decode as D
import Json.Encode as E
import Wire


type alias Group =
    { key : String, label : String, value : Float, weight : Float, samples : Int, evidence : List Evidence }


type alias Evidence =
    { subject : String, concernId : Maybe String, reference : Maybe String }


type alias Controls msg =
    { measure : E.Value -> msg
    , dimension : String -> msg
    , choose : String -> msg
    , evidence : Evidence -> Html msg
    }


groups : D.Value -> List Group
groups body =
    let
        decoder =
            D.map6 Group
                (D.field "key" D.string)
                (D.field "label" D.string)
                (D.field "value" D.float)
                (D.field "weight" D.float)
                (D.field "samples" D.int)
                (D.field "evidence" (D.list evidenceDecoder))
    in
    D.decodeValue (D.field "groups" (D.list decoder)) body |> Result.withDefault []


evidenceDecoder : D.Decoder Evidence
evidenceDecoder =
    D.map3 Evidence
        (D.field "subject" D.string)
        (D.field "concern_id" (D.nullable D.string))
        (D.field "reference" (D.nullable D.string))


filters : D.Value -> { measure : E.Value, dimension : String, snapshot : String, drilled : Bool }
filters body =
    { measure = Wire.field "measure" body
    , dimension = Wire.str "group_by" body
    , snapshot = Wire.str "id" (Wire.field "source" body)
    , drilled = Wire.str "bucket" body /= ""
    }


chart : Maybe String -> D.Value -> E.Value
chart active body =
    E.object
        [ ( "id", E.string "code-map" )
        , ( "type", E.string "treemap" )
        , ( "selected", Maybe.map E.string active |> Maybe.withDefault E.null )
        , ( "tiles", E.list (tile (Wire.str "unit" body)) (groups body) )
        ]


tile : String -> Group -> E.Value
tile unit group =
    E.object
        [ ( "id", E.string group.key )
        , ( "label", E.string group.label )
        , ( "value", E.string (String.fromFloat group.value ++ " " ++ unit) )
        , ( "weight", E.float group.weight )
        ]


view : Controls msg -> Maybe String -> D.Value -> List (Html msg)
view controls active body =
    let
        entries = groups body
        zero = List.filter (\group -> group.weight == 0) entries
    in
    [ selectors controls body
    , p [ class "code-map-caption" ] [ text (caption body) ]
    , if List.isEmpty entries then
        p [ class "panel-empty" ] [ text "No observations for this selection." ]
      else if List.any (\group -> group.weight > 0) entries then
        div [ attribute "id" "code-map", class "chart code-map-chart" ] []
      else
        text ""
    , if List.isEmpty zero then text "" else
        div [ class "code-map-controls" ] (span [] [ text "Zero-weight groups" ] :: List.map (zeroGroup controls.choose) zero)
    , entries |> List.filter (\group -> Just group.key == active) |> List.head
        |> Maybe.map (details controls.evidence (Wire.str "unit" body))
        |> Maybe.withDefault (text "")
    ]


selectors : Controls msg -> D.Value -> Html msg
selectors controls body =
    let
        active = filters body
        measures = D.decodeValue (D.field "measures" (D.list D.value)) body |> Result.withDefault []
        dimensions = D.decodeValue (D.field "dimensions" (D.list D.string)) body |> Result.withDefault []
        encode = E.encode 0
        changeMeasure raw =
            controls.measure (D.decodeString D.value raw |> Result.withDefault active.measure)
    in
    div [ class "code-map-controls" ]
        [ picker "Measure" changeMeasure (encode active.measure) (List.map (\measure -> ( encode measure, measureName measure )) measures)
        , picker "Group by" controls.dimension active.dimension (List.map (\dimension -> ( dimension, dimension )) dimensions)
        ]


picker : String -> (String -> msg) -> String -> List ( String, String ) -> Html msg
picker title change current choices =
    label []
        [ span [] [ text title ]
        , select [ attribute "aria-label" title, onInput change ]
            (List.map (\( key, name ) -> option [ value key, selected (key == current) ] [ text name ]) choices)
        ]


measureName : D.Value -> String
measureName measure =
    case Wire.str "kind" measure of
        "encounters" -> "Reported concern encounters"
        "open_concerns" -> "Open concerns"
        _ -> Wire.str "producer" measure ++ " / " ++ Wire.str "measure" measure


caption : D.Value -> String
caption body =
    let
        source = Wire.field "source" body
        revision = Wire.str "revision" source
        size = if flag "size_by_samples" body then "Tile area shows sample count" else "Tile area shows " ++ Wire.str "unit" body
    in
    String.join " · " <|
        [ size, Wire.str "reducer" body ]
        ++ (if revision == "" then [] else [ "Revision " ++ revision, Wire.str "observed_at" source ])
        ++ (if flag "partial" body then [ "Showing a bounded set of observations or groups" ] else [])
        ++ (if flag "measures_partial" body then [ "Measure list is partial" ] else [])


flag : String -> D.Value -> Bool
flag key body =
    D.decodeValue (D.field key D.bool) body == Ok True


zeroGroup : (String -> msg) -> Group -> Html msg
zeroGroup choose group =
    button [ onClick (choose group.key) ] [ text group.label ]


details : (Evidence -> Html msg) -> String -> Group -> Html msg
details render unit group =
    div [ class "code-map-evidence", attribute "aria-live" "polite" ]
        [ p [] [ text (group.label ++ " · " ++ String.fromFloat group.value ++ " " ++ unit) ]
        , ul [] (List.map (\item -> li [] [ render item ]) group.evidence)
        , if group.samples > List.length group.evidence then
            p [ class "code-map-caption" ] [ text "Up to six distinct supporting records are shown." ]
          else text ""
        ]
