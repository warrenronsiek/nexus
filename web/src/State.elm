-- @feature observability-ui
-- @spec docs/features/observability-ui.md
module State exposing (Remote, accept, begin, empty)


type alias Remote a =
    { value : Maybe a, generation : Int, loading : Bool, error : Maybe String }


empty : Remote a
empty =
    { value = Nothing, generation = 0, loading = False, error = Nothing }


begin : Remote a -> Remote a
begin previous =
    { previous | generation = previous.generation + 1, loading = True, error = Nothing }


accept : Int -> Result String a -> Remote a -> Remote a
accept generation result previous =
    if generation /= previous.generation then
        previous

    else
        case result of
            Ok value ->
                { previous | value = Just value, loading = False, error = Nothing }

            Err reason ->
                { previous | loading = False, error = Just reason }
