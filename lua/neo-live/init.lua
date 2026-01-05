local logger = require("neo-live.log")

logger.log("Connected to server")
logger.log({ port = 3248, status = "ok" }, "DEBUG")

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

-- Helper to decode 4-byte Big Endian Unsigned Integer
local function decode_u32(str)
    local b1, b2, b3, b4 = string.byte(str, 1, 4)
    return bit.bor(
        bit.lshift(b1, 24),
        bit.lshift(b2, 16),
        bit.lshift(b3, 8),
        b4
    )
end

function M.start_client()
    if M.client_proc then
        vim.notify("NeoLive Client is already connected!", vim.log.levels.WARN)
        return
    end

    local cmd = {
        M.config.binary_path,
        "--port", tostring(M.config.port),
        "connect",
        "--address", M.config.address,
    }

    -- 1. Persistent buffer to hold partial chunks across callbacks
    M.client_proc = vim.system(cmd, {
        stdin = true,
        stdout = function(_, data)
            if not data then return end

            -- 2. Append new chunk to our persistent buffer
            -- vim.schedule(function()
            --     vim.api.nvim_buf_set_lines(0, -1, -1, false, { type(data), tostring(data) })
            -- end)

            local len = decode_u32(data)
            logger.log("len: " .. tostring(len))

            -- 4. Extract the payload
            -- Lua string indices are 1-based.
            -- Header is 1..4, Payload is 5..(4+len)
            local payload_str = string.sub(data, 5, 4 + len)

            -- 5. Decode MsgPack
            local success, decoded = pcall(vim.mpack.decode, payload_str)
            logger.log("text: " .. vim.inspect(decoded))

            if success then
                vim.schedule(function()
                    if decoded.text then
                        logger.log("Updating buffer with text length: " .. #decoded.text, "DEBUG")
                        local lines = vim.split(decoded.text, "\n")
                        vim.api.nvim_buf_set_lines(0, 0, -1, false, lines)
                    end
                end)
            else
                -- If decoding fails, we likely have protocol garbage.
                -- In a real app, you might want to reset the buffer or log error.
                vim.schedule(function() logger.log("MsgPack decode failed", "ERROR") end)
            end
        end,
        --[[ debug
        stderr = function(err, data)
            vim.schedule(function()
                local lines = vim.split(data, "\n")
                vim.api.nvim_buf_set_lines(0, -1, -1, false, {"stderr"})
                vim.api.nvim_buf_set_lines(0, -1, -1, false, lines)
            end)
        end,
        ]]
        text = false, -- CRITICAL: Ensures we get raw bytes, not text lines
    }, function(obj)
        M.client_proc = nil
        vim.schedule(function()
            local level = obj.code == 0 and vim.log.levels.INFO or vim.log.levels.ERROR
            vim.notify("NeoLive Client disconnected. Code: " .. obj.code, level)
        end)
    end)

    print("NeoLive Client connecting to " .. M.config.address .. ":" .. M.config.port)
end

-- stop all processes
function M.stop()
    if M.client_proc then
        M.server_proc:kill(9)
        M.server_proc = nil
    end

    if M.server_proc then
        M.server_proc:kill(9)
        M.server_proc = nil
    end
end

return M
