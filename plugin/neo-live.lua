if vim.g.loaded_neo_live then return end
vim.g.loaded_neo_live = 1

vim.api.nvim_create_user_command("LiveServe", function()
    require("neo-live").start_server()
end, {})

vim.api.nvim_create_user_command("LiveConnect", function()
    require("neo-live").start_client()
end, {})
