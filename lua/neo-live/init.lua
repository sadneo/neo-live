local log = require("neo-live.log")

local M = {}
M.config = {
    address = "127.0.0.1",
    port = "3248",
    binary_path = vim.fn.getcwd() .. "/target/debug/neo-live", -- TMP
}

M._client_job = nil
M._is_applying_remote = false
M._managed_buffers = {}
M._known_buffers = {}
M._sent_open = false
M._pending_updates = {}
M._prompting_buffers = {}

-- Get all lines from the current buffer (0)
local function get_buffer_text()
    local lines = vim.api.nvim_buf_get_lines(0, 0, -1, false)
    return table.concat(lines, "\n")
end

local function get_buffer_text_for(bufnr)
    local lines = vim.api.nvim_buf_get_lines(bufnr, 0, -1, false)
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

local function normalize_buffer_name(name)
    if name == "" then return "" end
    return vim.fn.fnamemodify(name, ":.")
end

local function send_plugin_open(buffers)
    if not M._client_job then return end
    local payload = vim.mpack.encode({ buffers = buffers })
    local length = encode_length(#payload)
    M._client_job:write(length .. payload)
end

local function collect_open_buffers()
    local buffers = {}
    for _, buf in ipairs(vim.api.nvim_list_bufs()) do
        local name = vim.api.nvim_buf_get_name(buf)
        local normalized = normalize_buffer_name(name)
        if normalized ~= "" then
            if not M._known_buffers[normalized] then
                M._known_buffers[normalized] = true
                table.insert(buffers, normalized)
            end
        end
    end
    return buffers
end

local function attach_buffer(bufnr)
    vim.api.nvim_buf_attach(bufnr, false, {
        on_lines = function()
            if M._is_applying_remote or not M._client_job or not M._sent_open then return end

            local buffer = vim.api.nvim_buf_get_name(bufnr)
            local normalized = normalize_buffer_name(buffer)
            if normalized == "" then
                log.log("Skipping PluginUpdate: empty buffer name", "WARN")
                return
            end

            local cursor = { 0, 0 }
            if bufnr == vim.api.nvim_get_current_buf() then
                cursor = vim.api.nvim_win_get_cursor(0)
            end

            local text = get_buffer_text_for(bufnr)
            log.log(
                string.format("Sending PluginUpdate buffer=%s len=%d", normalized, #text),
                "TRACE"
            )

            local payload = vim.mpack.encode({
                cursor_row = cursor[1],
                cursor_col = cursor[2],
                buffer = normalized,
                text = text,
            })

            local length = encode_length(#payload)
            M._client_job:write(length .. payload)
        end
    })
end

function M.setup(opts)
    M.config = vim.tbl_deep_extend("force", M.config, opts or {})
end

function M.connect()
    if M._client_job then return end

    local bin = vim.fn.exepath(M.config.binary_path)
    if bin == "" then return print("Binary not found!") end

    log.log("Connecting to " .. M.config.address .. ":" .. M.config.port)

    local buffer = ""
    local function onClientUpdate(err, data)
        vim.schedule(function()
            if err then
                print("STDOUT ERROR: "..err)
            end
        end)

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
            if ok and decoded and type(decoded) == "table" and decoded.text and decoded.buffer then
                local bufname = decoded.buffer
                local lines = vim.split(decoded.text, "\n", true)
                vim.schedule(function()
                    if decoded.text == "" and not M._managed_buffers[bufname] then
                        return
                    end
                    if not M._managed_buffers[bufname] then
                        M._pending_updates[bufname] = lines
                        if M._prompting_buffers[bufname] then
                            return
                        end

                        M._prompting_buffers[bufname] = true
                        local prompt = string.format(
                            "neo-live has remote contents for %s. Overwrite buffer?",
                            bufname
                        )
                        local choice = vim.fn.confirm(prompt, "&Yes\n&No")
                        M._prompting_buffers[bufname] = nil

                        if choice ~= 1 then
                            M._pending_updates[bufname] = nil
                            return
                        end

                        M._managed_buffers[bufname] = true
                        lines = M._pending_updates[bufname] or lines
                        M._pending_updates[bufname] = nil
                    end

                    -- WARN: edits that happen between here will fall through the cracks,
                    -- might need a better solution
                    M._is_applying_remote = true
                    local bufnr = vim.fn.bufnr(bufname, true)
                    vim.api.nvim_buf_set_lines(bufnr, 0, -1, false, lines)
                    M._is_applying_remote = false
                end)
            else
                log.log("failed to decode", "ERROR")
            end
        end
    end

    -- spawn neo-live client and listen for updates
    M._client_job = vim.system(
        { bin, "--port", M.config.port, "connect", "--address", M.config.address },
        {
            stdin = true,
            stdout = onClientUpdate,
            stderr = function(_, data)
                if data then log.inner(data) end
            end,
        },
        function(obj)
            log.log("Client exited with code " .. obj.code)
            M._client_job = nil
            M._sent_open = false
        end
    )

    local open_buffers = collect_open_buffers()
    send_plugin_open(open_buffers)
    M._sent_open = true

    -- listen for buffer changes
    for _, buf in ipairs(vim.api.nvim_list_bufs()) do
        local name = vim.api.nvim_buf_get_name(buf)
        if normalize_buffer_name(name) ~= "" then
            attach_buffer(buf)
        end
    end

    vim.api.nvim_create_autocmd({ "BufReadPost", "BufNewFile" }, {
        callback = function(args)
            if not M._client_job or not M._sent_open then return end
            local name = vim.api.nvim_buf_get_name(args.buf)
            local normalized = normalize_buffer_name(name)
            if normalized == "" or M._known_buffers[normalized] then return end
            M._known_buffers[normalized] = true
            send_plugin_open({ normalized })
            attach_buffer(args.buf)
        end
    })
end

function M.stop()
    if M._client_job then
        M._client_job:kill(9)
        M._client_job = nil
    end
    M._sent_open = false
end

return M
