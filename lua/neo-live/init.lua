local M = {}

M.config = {
    port = 3248,
    binary_path = vim.fn.getcwd() .. "/target/debug/neo-live",
    address = "127.0.0.1",
}

M.server_proc = nil
M.client_proc = nil

function M.setup(opts)
    M.config = vim.tbl_deep_extend("force", M.config, opts or {})
end

-- frame creator (4 length prefix bytes, then msgpack)
local function make_packet(text)
    local payload = vim.mpack.encode({ text = text })
    local len = #payload

    -- big endian
    local header = string.char(
        bit.rshift(len, 24),
        bit.band(bit.rshift(len, 16), 0xFF),
        bit.band(bit.rshift(len, 8), 0xFF),
        bit.band(len, 0xFF)
    )

    return header .. payload
end

local function send_buffer_update(proc)
    if not proc then return end

    -- get all text from the current buffer
    local lines = vim.api.nvim_buf_get_lines(0, 0, -1, false)
    local full_text = table.concat(lines, "\n")

    -- wrap it and send to the process stdin
    local packet = make_packet(full_text)
    proc:write(packet)
end

function M.start_server()
    if M.server_proc then
        vim.notify("NeoLive Server is already running!", vim.log.levels.WARN)
        return
    end

    -- command: neo-live --port 3248 serve
    local cmd = {
        M.config.binary_path,
        "--port", tostring(M.config.port),
        "--log-output", "file",
        "--log-level", "trace",
        "serve"
    }

    -- process
    M.server_proc = vim.system(cmd, {
        stdin = true,
    }, function(obj)
        -- called on exit:
        M.server_proc = nil
        vim.schedule(function()
            local level = obj.code == 0 and vim.log.levels.INFO or vim.log.levels.ERROR
            vim.notify("NeoLive Server stopped. Code: " .. obj.code, level)
        end)
    end)

    vim.notify("NeoLive Hosting on port " .. M.config.port, vim.log.levels.INFO)

    -- autocmd
    vim.api.nvim_create_autocmd({ "TextChanged", "TextChangedI" }, {
        pattern = "*",
        callback = function()
            if M.server_proc then
                send_buffer_update(M.server_proc)
            end
        end,
    })

    send_buffer_update(M.server_proc)
end

function M.stop_server()
    if M.server_proc then
        M.server_proc:kill(9)
        M.server_proc = nil
    end
end

return M
