if vim.g.loaded_neo_live then return end
vim.g.loaded_neo_live = 1

vim.api.nvim_create_user_command("LiveConnect", function()
    require("neo-live").connect()
end, {})
