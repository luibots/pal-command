-- ===============================================================
-- PalFOV - custom FOV + on-screen map coordinates
-- Part of PAL COMMAND. Hot-reload with F10 (Restart All Mods).
--
-- Install: copy the PalFOV folder into
--   Palworld\Pal\Binaries\Win64\ue4ss\Mods\
-- and make sure it has an empty enabled.txt beside this Scripts folder.
-- ===============================================================

local FOV_TARGET = 150.0    -- change me, press F10 in the UE4SS console to apply
local SHOW_COORDS = true

-- World units -> in-game map coordinates.
-- Calibrated 2026-07-22 against a player at a known spot (reported 252,-502;
-- this yields 250.5,-501.2). Careful: the offsets transpose easily -
-- X uses 158000, Y uses 123000.
local function worldToMap(x, y)
    local mapX = (y - 158000.0) / 460.0
    local mapY = (x + 123000.0) / 460.0
    return mapX, mapY
end

local function getPC()
    local pc = FindFirstOf("PalPlayerController")
    if pc and pc:IsValid() then return pc end
    pc = FindFirstOf("PlayerController")
    if pc and pc:IsValid() then return pc end
    return nil
end

-- FOV: re-assert every second (the game resets the camera on mount/glide/menus)
LoopAsync(1000, function()
    local pc = getPC()
    if pc then
        local pcm = pc.PlayerCameraManager
        if pcm and pcm:IsValid() then
            pcm:SetFOV(FOV_TARGET)
        end
    end
    return false -- false = keep looping
end)

-- Coordinates on the HUD
RegisterHook("/Script/Engine.HUD:ReceiveDrawHUD", function(Context)
    if not SHOW_COORDS then return end
    local hud = Context:get()
    if not hud or not hud:IsValid() then return end
    local pc = hud.PlayerOwner
    if not pc or not pc:IsValid() then return end
    local pawn = pc.Pawn
    if not pawn or not pawn:IsValid() then return end

    local ok, loc = pcall(function() return pawn:K2_GetActorLocation() end)
    if not ok or not loc then return end

    local mapX, mapY = worldToMap(loc.X, loc.Y)
    local line1 = string.format("MAP  %d, %d", math.floor(mapX + 0.5), math.floor(mapY + 0.5))
    local line2 = string.format("RAW  %d / %d / %d", math.floor(loc.X), math.floor(loc.Y), math.floor(loc.Z))

    hud:DrawText(line1, { R = 1.0, G = 0.65, B = 0.14, A = 1.0 }, 26.0, 30.0, nil, 1.6, false)
    hud:DrawText(line2, { R = 0.75, G = 0.75, B = 0.80, A = 0.8 }, 26.0, 52.0, nil, 1.0, false)
end)

print("[PalFOV] loaded - FOV " .. FOV_TARGET .. ", coords " .. tostring(SHOW_COORDS) .. "\n")
