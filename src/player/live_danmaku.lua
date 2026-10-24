-- Render live Bilibili danmaku in one persistent MPV OSD layer.

local mp = require "mp"
local utils = require "mp.utils"

local MAX_ACTIVE = 320
local MAX_PENDING = 512
local config = {
    enabled = true, display_area = 0.5, opacity = 1.0, font_scale = 1.0,
    duration = 7.0, stroke_width = 2.0, line_height = 1.6,
    massive_mode = false, font_family = "sans-serif",
}

local active, pending, lane_ready = {}, {}, {}
local next_lane = 1
local overlay = mp.create_osd_overlay("ass-events")
overlay.z = 20
local refresh_timer = nil
local fps_probe_timer = nil

-- Danmaku move on a clock that stops while playback is paused, buffering or
-- seeking, so comments stay with the video frame they belong to.
local idle_since = nil
local idle_total = 0

local function clock()
    return (idle_since or mp.get_time()) - idle_total
end

local function set_idle(idle)
    local now = mp.get_time()
    if idle and idle_since == nil then
        idle_since = now
    elseif not idle and idle_since ~= nil then
        idle_total = idle_total + now - idle_since
        idle_since = nil
    end
end

local function clamp(value, low, high)
    return math.max(low, math.min(high, value))
end

local function utf8_count(text)
    local _, count = text:gsub("[^\128-\191]", "")
    return count
end

-- libass has no escape for a backslash, so `\\N` would still be a line
-- break. Like mpv's own OSD, follow every backslash with U+2060 WORD JOINER
-- so no override tag can form, then escape the braces.
local WORD_JOINER = "\226\129\160"

local function escape_ass(text)
    local escaped = text:gsub("\\", "\\" .. WORD_JOINER)
        :gsub("{", "\\{"):gsub("}", "\\}"):gsub("[\r\n]", " ")
    return escaped
end

local function safe_font_name(text)
    return tostring(text or "sans-serif"):gsub("[\\{}]", "")
end

local function ass_color(value)
    local color = clamp(tonumber(value) or 0xFFFFFF, 0, 0xFFFFFF)
    local red = math.floor(color / 65536) % 256
    local green = math.floor(color / 256) % 256
    local blue = color % 256
    return string.format("%02X%02X%02X", blue, green, red)
end

local function layout(width, height)
    local font_size = math.max(18, math.floor(height / 30 * config.font_scale))
    local lane_height = math.max(font_size + 4, math.floor(font_size * config.line_height))
    local top = math.floor(height * 0.04)
    local usable = math.max(lane_height, math.floor(height * config.display_area) - top)
    return font_size, lane_height, top, math.max(1, math.floor(usable / lane_height))
end

local function select_lane(now, lanes)
    for lane = 1, lanes do
        if (lane_ready[lane] or 0) <= now then return lane end
    end
    if config.massive_mode then
        local lane = ((next_lane - 1) % lanes) + 1
        next_lane = lane % lanes + 1
        return lane
    end
    return nil
end

local function schedule(width, height, now)
    local font_size, lane_height, top, lanes = layout(width, height)
    local pixels_per_second = width / config.duration
    local safety_gap = math.max(40, font_size * 2)
    while #pending > 0 and #active < MAX_ACTIVE do
        local lane = select_lane(now, lanes)
        if lane == nil then break end
        local message = table.remove(pending, 1)
        local characters = math.max(1, utf8_count(message.text))
        -- CJK glyphs are approximately one em wide. The former 0.62-em
        -- estimate released a lane while bold Chinese text was still visible,
        -- allowing the following comment to catch and overlap it.
        local text_width = math.max(font_size * 2, characters * font_size * 1.05)
        message.created = now
        message.lane = lane
        message.text_width = text_width
        message.duration = (width + text_width + safety_gap) / pixels_per_second
        lane_ready[lane] = now + (text_width + safety_gap) / pixels_per_second
        active[#active + 1] = message
    end
    return font_size, lane_height, top, lanes
end


local function render()
    local width, height = mp.get_osd_size()
    if width <= 0 or height <= 0 then return end
    if not config.enabled then
        overlay.data = ""
        overlay:update()
        return
    end
    if #active == 0 and #pending == 0 and overlay.data == "" then
        return
    end

    local now = clock()
    local font_size, lane_height, top, lanes = schedule(width, height, now)
    local alpha = math.floor((1.0 - config.opacity) * 255 + 0.5)
    local lines, remaining = {}, {}
    for _, message in ipairs(active) do
        local age = now - message.created
        if age < message.duration then
            local progress = clamp(age / message.duration, 0, 1)
            local safety_gap = math.max(40, font_size * 2)
            local x = math.floor(width - (width + message.text_width + safety_gap) * progress)
            local lane = ((message.lane - 1) % lanes) + 1
            local y = top + (lane - 1) * lane_height
            local tags = string.format(
                "{\\an7\\pos(%d,%d)\\fn%s\\b1\\fs%d\\bord%.1f\\shad1\\alpha&H%02X&\\c&H%s&}",
                x, y, safe_font_name(config.font_family), font_size,
                config.stroke_width, alpha, ass_color(message.color)
            )
            lines[#lines + 1] = tags .. escape_ass(message.text)
            remaining[#remaining + 1] = message
        end
    end
    active = remaining
    overlay.res_x, overlay.res_y = width, height
    overlay.data = table.concat(lines, "\n")
    overlay:update()
end

local function enqueue_message(message)
    if type(message) ~= "table" or type(message.text) ~= "string" then return end
    local text = message.text:gsub("[\r\n]", " ")
    if text == "" then return end
    pending[#pending + 1] = { text = text, color = message.color }
    while #pending > MAX_PENDING do table.remove(pending, 1) end
end

local function on_danmaku(payload)
    if not config.enabled then return end
    enqueue_message(utils.parse_json(payload or ""))
end

local function on_danmaku_batch(payload)
    if not config.enabled then return end
    local messages = utils.parse_json(payload or "")
    if type(messages) ~= "table" then return end
    for _, message in ipairs(messages) do
        enqueue_message(message)
    end
end

local function on_config(payload)
    local value = utils.parse_json(payload or "")
    if type(value) ~= "table" then return end
    if type(value.enabled) == "boolean" then config.enabled = value.enabled end
    if type(value.massive_mode) == "boolean" then config.massive_mode = value.massive_mode end
    if type(value.font_family) == "string" then config.font_family = value.font_family end
    config.display_area = clamp(tonumber(value.display_area) or config.display_area, 0.1, 1.0)
    config.opacity = clamp(tonumber(value.opacity) or config.opacity, 0.0, 1.0)
    config.font_scale = clamp(tonumber(value.font_scale) or config.font_scale, 0.5, 2.5)
    config.duration = clamp(tonumber(value.duration) or config.duration, 3.0, 20.0)
    config.stroke_width = clamp(tonumber(value.stroke_width) or config.stroke_width, 0.0, 5.0)
    config.line_height = clamp(tonumber(value.line_height) or config.line_height, 1.0, 3.0)
    lane_ready = {}
    if not config.enabled then active, pending = {}, {} end
    render()
end

-- Comments on screen belong to the old position after a seek or a new file.
local function clear_danmaku()
    active, pending, lane_ready = {}, {}, {}
    render()
end

mp.observe_property("core-idle", "bool", function(_, idle)
    set_idle(idle == true)
end)
mp.register_event("seek", clear_danmaku)
mp.register_event("start-file", clear_danmaku)

mp.register_script_message("danmaku", on_danmaku)
mp.register_script_message("danmaku-batch", on_danmaku_batch)
mp.register_script_message("danmaku-config", on_config)
-- Start only after MPV reports the real display refresh rate, and recompute if
-- the window moves to another display. The video remains in audio-sync mode;
-- only the OSD cadence follows the display.
local function apply_display_fps(value)
    local display_fps = tonumber(value)
    if display_fps == nil or display_fps <= 0 then return false end
    -- Follow the actual display refresh rate exactly; the video source is
    -- still untouched and only the OSD danmaku is updated at this cadence.
    local fps = display_fps
    if refresh_timer == nil then
        refresh_timer = mp.add_periodic_timer(1 / fps, render)
    else
        refresh_timer.timeout = 1 / fps
        refresh_timer:resume()
    end
    if fps_probe_timer ~= nil then fps_probe_timer:stop() end
    return true
end

local function probe_display_fps()
    apply_display_fps(mp.get_property_number("display-fps", nil))
end

local function restart_fps_probe()
    if not apply_display_fps(mp.get_property_number("display-fps", nil))
        and fps_probe_timer ~= nil then
        fps_probe_timer:resume()
    end
end

fps_probe_timer = mp.add_periodic_timer(0.1, probe_display_fps)
mp.observe_property("display-fps", "native", function(_, value)
    if not apply_display_fps(value) then fps_probe_timer:resume() end
end)
mp.register_event("file-loaded", restart_fps_probe)
mp.register_event("video-reconfig", restart_fps_probe)
mp.add_timeout(0, restart_fps_probe)
mp.register_event("shutdown", function()
    if refresh_timer ~= nil then refresh_timer:kill() end
    if fps_probe_timer ~= nil then fps_probe_timer:kill() end
    overlay:remove()
end)
