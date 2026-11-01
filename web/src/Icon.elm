-- @feature observability-ui
-- @spec docs/features/observability-ui.md
-- Lucide 1.66.0, ISC license. See web/dist/THIRD_PARTY_LICENSES.txt.
module Icon exposing (Icon(..), view)

import Html exposing (Html)
import Html.Attributes exposing (attribute)
import Svg exposing (Svg)
import Svg.Attributes as A


type Icon
    = Activity
    | Branch
    | Flask
    | Tools
    | Book
    | Bug
    | Warning
    | Siren
    | Memory
    | Menu
    | Close
    | Back
    | Forward
    | Chevron
    | Refresh
    | Search
    | ZoomIn
    | ZoomOut
    | Logout
    | Lock
    | Layers
    | Inbox


view : Icon -> Html msg
view icon =
    Svg.svg
        [ A.viewBox "0 0 24 24", A.width "18", A.height "18"
        , A.fill "none", A.stroke "currentColor", A.strokeWidth "1.6"
        , A.strokeLinecap "round", A.strokeLinejoin "round"
        , attribute "aria-hidden" "true", attribute "focusable" "false", A.class "icon"
        ]
        (shapes icon)


shapes : Icon -> List (Svg msg)
shapes icon =
    case icon of
        Activity ->
            [ Svg.path [ A.d "M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2" ] [] ]

        Branch ->
            [ Svg.path [ A.d "M15 6a9 9 0 0 0-9 9V3" ] [], Svg.circle [ A.cx "18", A.cy "6", A.r "3" ] [], Svg.circle [ A.cx "6", A.cy "18", A.r "3" ] [] ]

        Flask ->
            [ Svg.path [ A.d "M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2" ] [], Svg.path [ A.d "M6.453 15h11.094" ] [], Svg.path [ A.d "M8.5 2h7" ] [] ]

        Tools ->
            [ Svg.path [ A.d "M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.106-3.105c.32-.322.863-.22.983.218a6 6 0 0 1-8.259 7.057l-7.91 7.91a1 1 0 0 1-2.999-3l7.91-7.91a6 6 0 0 1 7.057-8.259c.438.12.54.662.219.984z" ] [] ]

        Book ->
            [ Svg.path [ A.d "M12 5v16" ] [], Svg.path [ A.d "M20.001 19A2 2 0 0022 17V5a2 2 0 00-1.999-2L16 3.002A5 5 0 0012 5a5 5 0 00-4-2H4a2 2 0 00-2 2v12a2 2 0 001.999 2H8a5 5 0 014 2 5 5 0 014-2z" ] [] ]

        Bug ->
            [ Svg.path [ A.d "M12 20v-9" ] [], Svg.path [ A.d "M14 7a4 4 0 0 1 4 4v3a6 6 0 0 1-12 0v-3a4 4 0 0 1 4-4z" ] [], Svg.path [ A.d "M14.12 3.88 16 2" ] [], Svg.path [ A.d "M21 21a4 4 0 0 0-3.81-4" ] [], Svg.path [ A.d "M21 5a4 4 0 0 1-3.55 3.97" ] [], Svg.path [ A.d "M22 13h-4" ] [], Svg.path [ A.d "M3 21a4 4 0 0 1 3.81-4" ] [], Svg.path [ A.d "M3 5a4 4 0 0 0 3.55 3.97" ] [], Svg.path [ A.d "M6 13H2" ] [], Svg.path [ A.d "m8 2 1.88 1.88" ] [], Svg.path [ A.d "M9 7.13V6a3 3 0 1 1 6 0v1.13" ] [] ]

        Warning ->
            [ Svg.path [ A.d "m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" ] [], Svg.path [ A.d "M12 9v4" ] [], Svg.path [ A.d "M12 17h.01" ] [] ]

        Siren ->
            [ Svg.path [ A.d "M7 18v-6a5 5 0 1 1 10 0v6" ] [], Svg.path [ A.d "M5 21a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-1a2 2 0 0 0-2-2H7a2 2 0 0 0-2 2z" ] [], Svg.path [ A.d "M21 12h1" ] [], Svg.path [ A.d "M18.5 4.5 18 5" ] [], Svg.path [ A.d "M2 12h1" ] [], Svg.path [ A.d "M12 2v1" ] [], Svg.path [ A.d "m4.929 4.929.707.707" ] [], Svg.path [ A.d "M12 12v6" ] [] ]

        Memory ->
            [ Svg.path [ A.d "M12 18V5" ] [], Svg.path [ A.d "M15 13a4.17 4.17 0 01-3-4 4.17 4.17 0 01-3 4" ] [], Svg.path [ A.d "M17.707 6.294A3 3 0 1012 5a3 3 0 10-5.707 1.294" ] [], Svg.path [ A.d "M17.996 5.125a4 4 0 012.526 5.77" ] [], Svg.path [ A.d "M18 18a4 4 0 002-7.465" ] [], Svg.path [ A.d "M19.967 17.482a4 4 0 11-7.967.517 4 4 0 11-7.967-.517" ] [], Svg.path [ A.d "M6 18a4 4 0 01-2-7.465" ] [], Svg.path [ A.d "M6.004 5.125a4 4 0 00-2.526 5.77" ] [] ]

        Menu ->
            [ Svg.path [ A.d "M4 5h16" ] [], Svg.path [ A.d "M4 12h16" ] [], Svg.path [ A.d "M4 19h16" ] [] ]

        Close ->
            [ Svg.path [ A.d "M18 6 6 18" ] [], Svg.path [ A.d "m6 6 12 12" ] [] ]

        Back ->
            [ Svg.path [ A.d "m12 19-7-7 7-7" ] [], Svg.path [ A.d "M19 12H5" ] [] ]

        Forward ->
            [ Svg.path [ A.d "M5 12h14" ] [], Svg.path [ A.d "m12 5 7 7-7 7" ] [] ]

        Chevron ->
            [ Svg.path [ A.d "m9 18 6-6-6-6" ] [] ]

        Refresh ->
            [ Svg.path [ A.d "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" ] [], Svg.path [ A.d "M21 3v5h-5" ] [], Svg.path [ A.d "M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" ] [], Svg.path [ A.d "M8 16H3v5" ] [] ]

        Search ->
            [ Svg.path [ A.d "m21 21-4.34-4.34" ] [], Svg.circle [ A.cx "11", A.cy "11", A.r "8" ] [] ]

        ZoomIn ->
            [ Svg.circle [ A.cx "11", A.cy "11", A.r "8" ] [], Svg.line [ A.x1 "21", A.x2 "16.65", A.y1 "21", A.y2 "16.65" ] [], Svg.line [ A.x1 "11", A.x2 "11", A.y1 "8", A.y2 "14" ] [], Svg.line [ A.x1 "8", A.x2 "14", A.y1 "11", A.y2 "11" ] [] ]

        ZoomOut ->
            [ Svg.circle [ A.cx "11", A.cy "11", A.r "8" ] [], Svg.line [ A.x1 "21", A.x2 "16.65", A.y1 "21", A.y2 "16.65" ] [], Svg.line [ A.x1 "8", A.x2 "14", A.y1 "11", A.y2 "11" ] [] ]

        Logout ->
            [ Svg.path [ A.d "m16 17 5-5-5-5" ] [], Svg.path [ A.d "M21 12H9" ] [], Svg.path [ A.d "M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" ] [] ]

        Lock ->
            [ Svg.circle [ A.cx "12", A.cy "16", A.r "1" ] [], Svg.rect [ A.x "3", A.y "10", A.width "18", A.height "12", A.rx "2" ] [], Svg.path [ A.d "M7 10V7a5 5 0 0 1 10 0v3" ] [] ]

        Layers ->
            [ Svg.path [ A.d "M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z" ] [], Svg.path [ A.d "M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12" ] [], Svg.path [ A.d "M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17" ] [] ]

        Inbox ->
            [ Svg.polyline [ A.points "22 12 16 12 14 15 10 15 8 12 2 12" ] [], Svg.path [ A.d "M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" ] [] ]
