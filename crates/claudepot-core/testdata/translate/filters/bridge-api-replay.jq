# Extracted from clai tests/bridge-api.sh mock claude: tool_result replay request.
{stream:$stream,max_tokens:512,messages:[{role:"user",content:"read"},{role:"assistant",content:$prior[0].content},{role:"user",content:[$prior[0].content[]|select(.type=="tool_use")|{type:"tool_result",tool_use_id:.id,content:"file data"}]}]}
