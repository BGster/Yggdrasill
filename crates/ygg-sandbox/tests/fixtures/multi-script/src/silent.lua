-- 无返回值模块：require 缓存为 `true`，同样只执行一次（负向面：不得执行两次）。
silent_runs = (silent_runs or 0) + 1
load_trace = (load_trace or "") .. ">silent"
