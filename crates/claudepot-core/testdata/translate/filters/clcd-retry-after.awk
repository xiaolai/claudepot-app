# Extracted from clai clcd upstream_error(): Retry-After header.
tolower($0) ~ /^retry-after:/ {sub(/^[^:]*:[ ]*/,"");sub(/\r$/,"");v=$0} END{print v}
