local M = {}

-- 1. Determine safe log path
-- Linux: ~/.local/state/nvim/neo-live.log
-- Mac: ~/Library/Application Support/nvim/neo-live.log
local path = vim.fn.stdpath("state") .. "/neo-live.log"

-- log inner process's stderr distinctly
function M.inner(msg)
    local f = io.open(path, "a") -- "a" for append
    if f then
        f:write(string.format("[client] %s", msg))
        f:close()
    end
end

function M.log(msg, level)
    local f = io.open(path, "a") -- "a" for append
    if f then
        local timestamp = os.date("%H:%M:%S")

        -- If msg is a table, dump it nicely
        if type(msg) == "table" then
            msg = vim.inspect(msg)
        end

        local str = string.format("[plugin] [%s] [%s] %s\n", timestamp, level or "INFO", msg)
        f:write(str)
        f:close()
    end
end

-- Optional: Clear log on startup
function M.clear()
    local f = io.open(path, "w")
    if f then f:close() end
end

return M
