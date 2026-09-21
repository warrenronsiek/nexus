-- @feature observability-ui
-- @spec docs/features/observability-ui.md
-- @boundary dynamic-http-json
module Nexus.Api exposing (fetchDashboard, fetchProjects, httpError)

import Http
import Nexus.Domain exposing (Dashboard, ProjectScope(..), ProjectSummary, dashboardDecoder, projectsDecoder)
import Url


fetchProjects : (Result Http.Error (List ProjectSummary) -> message) -> Cmd message
fetchProjects toMessage =
    Http.get
        { url = "/api/v1/projects"
        , expect = Http.expectJson toMessage projectsDecoder
        }


fetchDashboard : ProjectScope -> (Result Http.Error Dashboard -> message) -> Cmd message
fetchDashboard scope toMessage =
    Http.get
        { url = dashboardUrl scope
        , expect = Http.expectJson toMessage dashboardDecoder
        }


dashboardUrl : ProjectScope -> String
dashboardUrl scope =
    case scope of
        AllProjects ->
            "/api/v1/dashboard"

        OneProject projectId ->
            "/api/v1/dashboard?project_id=" ++ Url.percentEncode projectId


httpError : Http.Error -> String
httpError error =
    case error of
        Http.BadUrl _ ->
            "The dashboard URL is invalid."

        Http.Timeout ->
            "The dashboard request timed out."

        Http.NetworkError ->
            "The Nexus daemon is unreachable."

        Http.BadStatus status ->
            "The dashboard returned HTTP " ++ String.fromInt status ++ "."

        Http.BadBody message ->
            "The dashboard returned unexpected data: " ++ message
