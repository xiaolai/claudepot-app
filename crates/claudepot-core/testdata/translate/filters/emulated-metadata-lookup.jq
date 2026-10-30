# NOT clai code. Emulates clcd prepare_chat() (clcd:327-343, bash): for each
# function_call in the normalized request, load the cached entry stored under
# (selected model, call id) — last write wins, as `mv -f` does — and require
# .id and .model to match. A missing entry is prepare_chat's `return 1`, which
# clcd turns into HTTP 400 (clcd:502-503).
# Input: the normalized Responses request. $cache: every cached entry, in write order.
.model as $model |
reduce (.input[]|select(.type=="function_call")|.call_id) as $id ({};
  ([$cache[]|select(.id==$id and .model==$model)]|last) as $entry |
  if $entry==null then error("no cached replay metadata for model "+$model+" and call "+$id)
  else .+{($id):$entry} end)
