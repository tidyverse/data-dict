-- {{< cli-version >}} prints the CLI version, read from the workspace
-- Cargo.toml at render time so the homepage never drifts from the release.
-- (Named cli-version because Quarto has a built-in {{< version >}} shortcode
-- that prints the Quarto version.)
local function workspace_version()
  -- Filters don't run with the site as the working directory, so anchor on
  -- the project root Quarto exports.
  local dir = os.getenv("QUARTO_PROJECT_DIR") or "."
  local f = io.open(dir .. "/../Cargo.toml", "r")
  if not f then
    return nil
  end
  for line in f:lines() do
    local v = line:match('^version%s*=%s*"([^"]+)"')
    if v then
      f:close()
      return v
    end
  end
  f:close()
  return nil
end

return {
  ["cli-version"] = function()
    return pandoc.Str(workspace_version() or "unknown")
  end,
}
