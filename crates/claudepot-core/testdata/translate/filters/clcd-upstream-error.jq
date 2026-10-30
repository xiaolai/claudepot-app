# Extracted from clai clcd upstream_error(): upstream error body -> Anthropic error.

    (try fromjson catch {}) as $e |
    (($e.error.message // $e.detail // $e.message // "Codex upstream request failed") | if type=="string" then . else tojson end |
      gsub("eyJ[A-Za-z0-9_.-]+|sk-[A-Za-z0-9_*.-]+|xai-[A-Za-z0-9_*.-]+|AIza[A-Za-z0-9_*.-]+";"<redacted>") | .[0:500]) as $message |
    {type:"error",error:{type:(if $status=="429" then "rate_limit_error" elif $status=="401" or $status=="403" then "authentication_error" elif ($status|startswith("4")) then "invalid_request_error" else "api_error" end),
      message:(if ($message|test("context.{0,20}(length|window|limit)|too many tokens|prompt is too long";"i")) then "prompt is too long: "+$message else $message end)}}
