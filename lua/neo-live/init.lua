local log = require("neo-live.log")

local M = {}
M.config = {
    address = "127.0.0.1",
    port = "3248",
    binary_path = vim.fn.getcwd() .. "/target/debug/neo-live",
}

M._client_job = nil
M._is_applying_remote = false

-- Get all lines from the current buffer (0)
local function get_buffer_text()
    local lines = vim.api.nvim_buf_get_lines(0, 0, -1, false)
    return table.concat(lines, "\n")
end

local function decode_length(chunk)
    if #chunk < 4 then return nil end

    -- 1. Extract the first 4 bytes as numbers (0-255)
    local b1, b2, b3, b4 = string.byte(chunk, 1, 4)

    -- 2. Shift them into position (Big Endian: MSB first)
    -- b1 is the most significant byte (bits 24-31)
    -- b4 is the least significant byte (bits 0-7)
    local len = bit.bor(
        bit.lshift(b1, 24),
        bit.lshift(b2, 16),
        bit.lshift(b3, 8),
        b4
    )

    return len
end

local function encode_length(len)
    return string.char(
        bit.rshift(len, 24),
        bit.rshift(bit.band(len, 0xFF0000), 16),
        bit.rshift(bit.band(len, 0xFF00), 8),
        bit.band(len, 0xFF)
    )
end

function M.setup(opts)
    M.config = vim.tbl_deep_extend("force", M.config, opts or {})
end

function M.connect()
    if M._client_job then return end

    local bin = vim.fn.exepath(M.config.binary_path)
    if bin == "" then return print("Binary not found!") end

    log.log("Connecting to " .. M.config.address .. ":" .. M.config.port)

    -- 1. Attach to Buffer Changes
    vim.api.nvim_buf_attach(0, false, {
        on_lines = function()
            -- don't register changes if it's from neo-live
            if M._is_applying_remote then return end

            -- CHECK: Ensure we have a running client job
            if M._client_job then
                local text = get_buffer_text()
                log.log("Sending buffer text " .. text, "TRACE")
                local payload = vim.mpack.encode({ text = text })

                local length = encode_length(#payload)
                M._client_job:write(length .. payload)
            end
        end
    })

    -- 2. Spawn Client with IO Callbacks
    local buffer = ""

    M._client_job = vim.system(
        { bin, "--port", M.config.port, "connect", "--address", M.config.address },
        {
            stdin = true,
            stdout = function(_, data)
                if not data then return end
                buffer = buffer .. data;
                log.log(string.format("received data of len %d, buffer len %d", #data, #buffer), "TRACE")

                while #buffer >= 4 do
                    local length = decode_length(buffer)
                    if #buffer < 4 + length then return end

                    log.log("decoding", "TRACE")
                    -- decode it, seems valid
                    local raw_payload = string.sub(buffer, 5, 4 + length)
                    buffer = string.sub(buffer, length + 5)

                    local ok, decoded = pcall(vim.mpack.decode, raw_payload)

                    log.log(string.format("decoded: %s, ok: %s", vim.inspect(decoded), tostring(ok)), "TRACE")
                    if ok and decoded and type(decoded) == "table" and decoded.text then
                        local lines = vim.split(decoded.text, "\n")
                        vim.schedule(function()
                            M._is_applying_remote = true
                            vim.api.nvim_buf_set_lines(0, 0, -1, false, lines)
                            M._is_applying_remote = false
                        end)
                    else
                        log.log("failed to decode", "ERROR")
                    end
                end
            end,
            stderr = function(_, data)
                if data then log.inner(data) end
            end,
        },
        function(obj)
            log.log("Client exited with code " .. obj.code)
            M._client_job = nil
        end
    )
end

function M.stop()
    if M._client_job then
        M._client_job:kill(9)
        M._client_job = nil
    end
end

return M
