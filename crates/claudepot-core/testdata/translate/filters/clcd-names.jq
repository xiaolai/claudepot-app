# Extracted from clai clcd handle(): wire tool names.
[(.tools // []) | to_entries[] | {key:.value.name,value:(if (.value.name|test("^[A-Za-z0-9_-]{1,64}$") and (startswith("clcd_tool_")|not)) then .value.name else "clcd_tool_"+(.key|tostring) end)}] | from_entries
