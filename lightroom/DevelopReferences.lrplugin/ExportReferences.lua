-- Builds the full set of Lightroom reference JPEGs for each selected photo:
--
--   <photo>.jpg                 the photo reset to Lightroom's defaults (blank reference)
--   <photo>-<suffix><NNN>.jpg   one slider moved from the blank state, e.g. DSC05615-b-050.jpg
--
-- Every file is exported with the settings of a Lightroom export preset ("raw reference" by
-- default), only the file name is set here. When it is done the photo gets its previous
-- settings back; every step is also in the History panel.

local LrApplication = import "LrApplication"
local LrApplicationView = import "LrApplicationView"
local LrBinding = import "LrBinding"
local LrDevelopController = import "LrDevelopController"
local LrDialogs = import "LrDialogs"
local LrExportSession = import "LrExportSession"
local LrFileUtils = import "LrFileUtils"
local LrFunctionContext = import "LrFunctionContext"
local LrPathUtils = import "LrPathUtils"
local LrPrefs = import "LrPrefs"
local LrProgressScope = import "LrProgressScope"
local LrTasks = import "LrTasks"
local LrView = import "LrView"

local PLUGIN = "Develop References"

-- The reference set. id: Lightroom develop setting; suffix: file name part; relative: the
-- value is added to the blank value (white balance follows the camera, so it has no zero).
local SLIDERS = {
    { id = "Exposure2012",   label = "Exposure",     suffix = "e",      values = "-2, -1, 1, 2",            on = false },
    { id = "Contrast2012",   label = "Contrast",     suffix = "c",      values = "-100, -50, 50, 100",      on = false },
    { id = "Highlights2012", label = "Highlights",   suffix = "h",      values = "-100, -50, 50, 100",      on = true },
    { id = "Shadows2012",    label = "Shadows",      suffix = "s",      values = "-100, -50, 50, 100",      on = true },
    { id = "Whites2012",     label = "Whites",       suffix = "w",      values = "-100, -50, 50, 100",      on = true },
    { id = "Blacks2012",     label = "Blacks",       suffix = "b",      values = "-100, -50, 50, 100",      on = true },
    { id = "Texture",        label = "Texture",      suffix = "text",   values = "-100, -50, 50, 100",      on = true },
    { id = "Clarity2012",    label = "Clarity",      suffix = "cla",    values = "-100, -50, 50, 100",      on = true },
    { id = "Dehaze",         label = "Dehaze",       suffix = "dehaze", values = "-100, -50, 50, 100",      on = true },
    { id = "Vibrance",       label = "Vibrance",     suffix = "v",      values = "-100, -50, 50, 100",      on = true },
    { id = "Saturation",     label = "Saturation",   suffix = "sat",    values = "-100, -50, 50, 100",      on = true },
    { id = "Sharpness",      label = "Sharpening",   suffix = "sha",    values = "0, 40, 75, 115, 150",     on = true },
    { id = "Temperature",    label = "Temp (+/- K)", suffix = "temp",   values = "-1000, -500, 500, 1000",  on = false, relative = true },
    { id = "Tint",           label = "Tint (+/-)",   suffix = "tint",   values = "-150, -75, 75, 150",      on = true,  relative = true },
    -- Tone curve: the four parametric regions, and named point curves (see POINT_CURVES).
    { id = "ParametricHighlights", label = "Curve Highlights", suffix = "ph", values = "-100, -50, 50, 100", on = false },
    { id = "ParametricLights",     label = "Curve Lights",     suffix = "pl", values = "-100, -50, 50, 100", on = false },
    { id = "ParametricDarks",      label = "Curve Darks",      suffix = "pd", values = "-100, -50, 50, 100", on = false },
    { id = "ParametricShadows",    label = "Curve Shadows",    suffix = "ps", values = "-100, -50, 50, 100", on = false },
    { id = "ToneCurvePV2012",      label = "Point curves",     suffix = "pc", values = "mid+32, mid-32, shadow+32, high-32, s, black+32", on = false, curves = true },
    -- Color Mixer: each row exports all eight colours, e.g. <photo>-hsl-s-blue-100.jpg.
    { id = "HueAdjustment",        label = "Mixer Hue (8)",        suffix = "hsl-h", values = "-100, 100", on = false, colours = true },
    { id = "SaturationAdjustment", label = "Mixer Saturation (8)", suffix = "hsl-s", values = "-100, 100", on = false, colours = true },
    { id = "LuminanceAdjustment",  label = "Mixer Luminance (8)",  suffix = "hsl-l", values = "-100, 100", on = false, colours = true },
}

-- The Color Mixer's colours, as Lightroom names its settings (HueAdjustmentRed...).
local COLOURS = { "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta" }

-- Point curves by name: x, y pairs on Lightroom's 0-255 scale.
local POINT_CURVES = {
    ["mid+32"] = { 0, 0, 128, 160, 255, 255 },
    ["mid-32"] = { 0, 0, 128, 96, 255, 255 },
    ["shadow+32"] = { 0, 0, 64, 96, 255, 255 },
    ["high-32"] = { 0, 0, 192, 160, 255, 255 },
    ["s"] = { 0, 0, 64, 48, 192, 208, 255, 255 },
    ["black+32"] = { 0, 32, 255, 255 },
}

local WHITE_BALANCE = { Temperature = true, Tint = true }

---------------------------------------------------------------------------------------------
-- Export preset

local function preset_candidates(name)
    local file = name .. ".lrtemplate"
    local list = {}
    local app_data = LrPathUtils.getStandardFilePath("appData")
    if app_data then
        -- appData may be Lightroom's own folder or the general application data folder.
        list[#list + 1] = LrPathUtils.child(LrPathUtils.child(LrPathUtils.child(app_data, "Export Presets"), "User Presets"), file)
        local adobe = LrPathUtils.child(LrPathUtils.child(app_data, "Adobe"), "Lightroom")
        list[#list + 1] = LrPathUtils.child(LrPathUtils.child(LrPathUtils.child(adobe, "Export Presets"), "User Presets"), file)
    end
    local home = LrPathUtils.getStandardFilePath("home")
    if home then
        local win = LrPathUtils.child(LrPathUtils.child(LrPathUtils.child(home, "AppData"), "Roaming"), "Adobe")
        list[#list + 1] = LrPathUtils.child(LrPathUtils.child(LrPathUtils.child(LrPathUtils.child(win, "Lightroom"), "Export Presets"), "User Presets"), file)
        local mac = LrPathUtils.child(LrPathUtils.child(LrPathUtils.child(home, "Library"), "Application Support"), "Adobe")
        list[#list + 1] = LrPathUtils.child(LrPathUtils.child(LrPathUtils.child(LrPathUtils.child(mac, "Lightroom"), "Export Presets"), "User Presets"), file)
    end
    return list
end

local function find_preset(name)
    for _, path in ipairs(preset_candidates(name)) do
        if LrFileUtils.exists(path) == "file" then return path end
    end
    return nil
end

local function parse_scalar(raw)
    local text = string.match(raw, '^"(.*)"$')
    if text then
        return (string.gsub(text, "\\(.)", function(c)
            if c == "n" then return "\n" end
            if c == "t" then return "\t" end
            return c
        end))
    end
    if raw == "true" then return true end
    if raw == "false" then return false end
    return tonumber(raw)
end

-- Reads the "value = { ... }" table of a .lrtemplate export preset (one key per line, as
-- Lightroom writes them) into LrExportSession settings (LR_ prefixed keys).
local function read_preset(path)
    local text = LrFileUtils.readFile(path)
    if not text then return nil, "cannot read " .. path end
    local settings, inside, depth, count = {}, false, 0, 0
    for line in string.gmatch(text, "[^\r\n]+") do
        if not inside then
            if string.match(line, "^%s*value%s*=%s*{%s*$") then inside = true end
        elseif depth > 0 then
            if string.match(line, "{%s*$") then depth = depth + 1 end
            if string.match(line, "^%s*}%s*,?%s*$") then depth = depth - 1 end
        elseif string.match(line, "^%s*}%s*,?%s*$") then
            break
        elseif string.match(line, "{%s*$") then
            depth = 1 -- nested table such as markedPresets: skipped
        else
            local key, raw = string.match(line, "^%s*([%w_]+)%s*=%s*(.-)%s*,%s*$")
            if key then
                local value = parse_scalar(raw)
                if value ~= nil then
                    settings["LR_" .. key] = value
                    count = count + 1
                end
            end
        end
    end
    if count == 0 then return nil, "no export settings found in " .. path end
    return settings
end

local function preset_folder(settings)
    local folder = settings.LR_export_destinationPathPrefix
    if folder and settings.LR_export_useSubfolder and settings.LR_export_destinationPathSuffix
        and settings.LR_export_destinationPathSuffix ~= "" then
        folder = LrPathUtils.child(folder, settings.LR_export_destinationPathSuffix)
    end
    return folder
end

---------------------------------------------------------------------------------------------
-- Develop settings

local function parse_curve_names(text)
    local names = {}
    for item in string.gmatch(text or "", "[^,%s]+") do
        if not POINT_CURVES[item] then return nil, "\"" .. item .. "\" is not a known point curve" end
        names[#names + 1] = item
    end
    return names
end

local function parse_values(text)
    local values = {}
    for item in string.gmatch(text or "", "[^,%s]+") do
        local v = tonumber(item)
        if v == nil then return nil, "\"" .. item .. "\" is not a number" end
        values[#values + 1] = v
    end
    return values
end

-- +050, -100, -000 (as in the existing reference files); decimals as +0.50.
local function format_value(v)
    if math.floor(v) == v then
        return (v > 0 and "+" or "-") .. string.format("%03d", math.abs(v))
    end
    return string.format("%+.2f", v)
end

local function close_enough(a, b)
    return type(a) == "number" and type(b) == "number" and math.abs(a - b) < 0.01
end

-- Lightroom applies settings asynchronously; wait until the photo reports them.
local function wait_for(photo, settings)
    for _ = 1, 100 do
        local now = photo:getDevelopSettings()
        local done = true
        for key, value in pairs(settings) do
            if type(value) == "number" and not close_enough(now[key], value) then done = false end
        end
        if done then return true end
        LrTasks.sleep(0.1)
    end
    return false
end

local function apply(catalog, photo, settings, history)
    catalog:withWriteAccessDo(history, function()
        photo:applyDevelopSettings(settings, history)
    end, { timeout = 60 })
    wait_for(photo, settings)
end

-- Resets the photo to Lightroom's defaults, as the Reset button in Develop does.
local function reset_to_defaults(catalog, photo)
    catalog:setSelectedPhotos(photo, { photo })
    LrApplicationView.switchToModule("develop")
    for _ = 1, 100 do
        if catalog:getTargetPhoto() == photo and LrApplicationView.getCurrentModuleName() == "develop" then break end
        LrTasks.sleep(0.1)
    end
    LrTasks.sleep(0.5)
    LrDevelopController.resetAllDevelopAdjustments()
    -- Wait until the reset shows in the photo's settings.
    for _ = 1, 100 do
        local now = photo:getDevelopSettings()
        if close_enough(now.Exposure2012 or 0, 0) and close_enough(now.Blacks2012 or 0, 0)
            and close_enough(now.Clarity2012 or 0, 0) and close_enough(now.Texture or 0, 0) then
            break
        end
        LrTasks.sleep(0.1)
    end
    -- Then until the settings stop changing (sliders that were already 0 tell us nothing).
    local last = nil
    for _ = 1, 20 do
        LrTasks.sleep(0.5)
        local now = photo:getDevelopSettings()
        local key = string.format("%s|%s|%s|%s|%s", tostring(now.Contrast2012), tostring(now.Highlights2012),
            tostring(now.Shadows2012), tostring(now.Tint), tostring(now.Sharpness))
        if key == last then return now end
        last = key
    end
    return photo:getDevelopSettings()
end

---------------------------------------------------------------------------------------------
-- Export

local function export_one(photo, preset, folder, name)
    local settings = {}
    for key, value in pairs(preset) do settings[key] = value end
    settings.LR_export_destinationType = "specificFolder"
    settings.LR_export_destinationPathPrefix = folder
    settings.LR_export_useSubfolder = false
    settings.LR_export_destinationPathSuffix = ""
    settings.LR_renamingTokensOn = true
    settings.LR_tokens = "{{custom_token}}"
    settings.LR_tokenCustomString = name
    settings.LR_collisionHandling = "overwrite"
    settings.LR_export_postProcessing = "doNothing"
    settings.LR_reimportExportedPhoto = false
    local session = LrExportSession { photosToExport = { photo }, exportSettings = settings }
    for _, rendition in session:renditions() do
        local ok, message = rendition:waitForRender()
        if not ok then error("Export of " .. name .. " failed: " .. tostring(message), 0) end
    end
end

-- The variants for one photo, given its blank (reset) settings.
local function variants(base, blank, chosen)
    local jobs = {}
    for _, slider in ipairs(chosen) do
        if slider.curves then
            for _, name in ipairs(slider.parsed) do
                jobs[#jobs + 1] = {
                    name = base .. "-pc-" .. name,
                    settings = { ToneCurveName2012 = "Custom", ToneCurvePV2012 = POINT_CURVES[name] },
                    undo = { ToneCurveName2012 = blank.ToneCurveName2012 or "Linear", ToneCurvePV2012 = blank.ToneCurvePV2012 or { 0, 0, 255, 255 } },
                    curve = true,
                }
            end
        elseif slider.colours then
            for _, colour in ipairs(COLOURS) do
                local id = slider.id .. colour
                for _, v in ipairs(slider.parsed) do
                    jobs[#jobs + 1] = {
                        name = base .. "-" .. slider.suffix .. "-" .. string.lower(colour) .. format_value(v),
                        settings = { [id] = v },
                        undo = { [id] = blank[id] or 0 },
                    }
                end
            end
        end
        for _, v in ipairs((slider.curves or slider.colours) and {} or slider.parsed) do
            local settings, undo = {}, {}
            if WHITE_BALANCE[slider.id] then
                settings.WhiteBalance = "Custom"
                settings.Temperature = blank.Temperature
                settings.Tint = blank.Tint
                undo.WhiteBalance = blank.WhiteBalance
                if blank.WhiteBalance == "Custom" then
                    undo.Temperature = blank.Temperature
                    undo.Tint = blank.Tint
                end
            end
            settings[slider.id] = slider.relative and ((blank[slider.id] or 0) + v) or v
            if not WHITE_BALANCE[slider.id] then
                undo[slider.id] = blank[slider.id] or 0
            end
            jobs[#jobs + 1] = { name = base .. "-" .. slider.suffix .. format_value(v), settings = settings, undo = undo }
        end
    end
    return jobs
end

local function run(context, props, chosen, preset)
    local catalog = LrApplication.activeCatalog()
    local photos = catalog:getTargetPhotos()
    local folder = props.folder
    if LrFileUtils.exists(folder) ~= "directory" then LrFileUtils.createAllDirectories(folder) end

    local per_photo = (props.export_blank and 1 or 0)
    for _, slider in ipairs(chosen) do per_photo = per_photo + #slider.parsed * (slider.colours and #COLOURS or 1) end
    local total = per_photo * #photos

    local progress = LrProgressScope { title = "Exporting reference JPEGs", functionContext = context }
    local done, exported, skipped = 0, 0, 0
    local problems = {}

    local function exists(name)
        return LrFileUtils.exists(LrPathUtils.child(folder, name .. ".jpg")) == "file"
    end

    for _, photo in ipairs(photos) do
        if progress:isCanceled() then break end
        local base = LrPathUtils.removeExtension(photo:getFormattedMetadata("fileName"))
        local original = photo:getDevelopSettings()
        progress:setCaption(base .. ": resetting to defaults")

        local ok, message = LrTasks.pcall(function()
            local blank = reset_to_defaults(catalog, photo)
            if props.export_blank then
                if exists(base) and not props.overwrite then
                    skipped = skipped + 1
                else
                    progress:setCaption(base .. ".jpg")
                    export_one(photo, preset, folder, base)
                    exported = exported + 1
                end
                done = done + 1
                progress:setPortionComplete(done, total)
            end
            for _, job in ipairs(variants(base, blank, chosen)) do
                if progress:isCanceled() then break end
                if exists(job.name) and not props.overwrite then
                    skipped = skipped + 1
                else
                    progress:setCaption(job.name .. ".jpg")
                    apply(catalog, photo, job.settings, "Reference " .. job.name)
                    if job.curve then LrTasks.sleep(1.5) end -- a point curve is a list: wait_for cannot check it
                    local exported_ok, export_message = LrTasks.pcall(export_one, photo, preset, folder, job.name)
                    apply(catalog, photo, job.undo, "Reference: back to blank")
                    if job.curve then LrTasks.sleep(1.5) end
                    if not exported_ok then error(export_message, 0) end
                    exported = exported + 1
                end
                done = done + 1
                progress:setPortionComplete(done, total)
            end
        end)
        if not ok then problems[#problems + 1] = base .. ": " .. tostring(message) end

        -- Give the photo its previous settings back.
        local restored = LrTasks.pcall(function()
            catalog:withWriteAccessDo("Reference: restore previous settings", function()
                photo:applyDevelopSettings(original, "Reference: restore previous settings")
            end, { timeout = 60 })
        end)
        if not restored then
            problems[#problems + 1] = base .. ": could not restore the previous settings; use the History panel"
        end
    end
    progress:done()

    local message = string.format("%d exported, %d skipped (already there) to\n%s", exported, skipped, folder)
    if #problems > 0 then
        message = message .. "\n\nProblems:\n" .. table.concat(problems, "\n")
    end
    LrDialogs.message(PLUGIN, message, #problems > 0 and "warning" or "info")
end

---------------------------------------------------------------------------------------------
-- Dialog

local function dialog(context)
    local catalog = LrApplication.activeCatalog()
    if #catalog:getTargetPhotos() == 0 then
        LrDialogs.message(PLUGIN, "Select the photo (or photos) to make references from first.", "warning")
        return
    end

    local prefs = LrPrefs.prefsForPlugin()
    local props = LrBinding.makePropertyTable(context)
    props.preset_name = prefs.preset_name or "raw reference"
    props.preset_path = prefs.preset_path or find_preset(props.preset_name) or ""
    props.folder = prefs.folder or ""
    props.export_blank = prefs.export_blank ~= false
    props.overwrite = prefs.overwrite ~= false
    for _, slider in ipairs(SLIDERS) do
        local saved = prefs[slider.id .. "_on"]
        if saved == nil then saved = slider.on end
        props[slider.id .. "_on"] = saved
        props[slider.id .. "_values"] = prefs[slider.id .. "_values"] or slider.values
    end
    if props.folder == "" and props.preset_path ~= "" then
        local preset = read_preset(props.preset_path)
        props.folder = (preset and preset_folder(preset)) or ""
    end

    local f = LrView.osFactory()
    local bind = LrView.bind
    local rows = {}
    for _, slider in ipairs(SLIDERS) do
        rows[#rows + 1] = f:row {
            f:checkbox { title = slider.label, value = bind(slider.id .. "_on"), width = LrView.share("slider_label") },
            f:edit_field { value = bind(slider.id .. "_values"), enabled = bind(slider.id .. "_on"), width_in_chars = 24 },
            f:static_text {
                title = slider.curves and "<photo>-pc-mid+32.jpg" or (slider.colours and ("<photo>-" .. slider.suffix .. "-blue+100.jpg") or ("<photo>-" .. slider.suffix .. "+050.jpg")),
                enabled = bind(slider.id .. "_on"),
            },
        }
    end

    local function choose_folder()
        local picked = LrDialogs.runOpenPanel {
            title = "Folder for the reference JPEGs",
            canChooseFiles = false, canChooseDirectories = true,
            canCreateDirectories = true, allowsMultipleSelection = false,
        }
        if picked and picked[1] then props.folder = picked[1] end
    end

    local function choose_preset()
        local picked = LrDialogs.runOpenPanel {
            title = "Export preset (.lrtemplate)",
            canChooseFiles = true, canChooseDirectories = false,
            allowsMultipleSelection = false, fileTypes = "lrtemplate",
        }
        if picked and picked[1] then props.preset_path = picked[1] end
    end

    local contents = f:column {
        bind_to_object = props,
        spacing = f:control_spacing(),
        f:static_text {
            title = "For each selected photo: resets it to Lightroom's defaults, exports the blank reference,\n"
                .. "then one JPEG per slider value. The photo gets its previous settings back at the end.",
        },
        f:group_box { title = "Sliders and values", fill_horizontal = 1, f:column(rows) },
        f:group_box {
            title = "Export", fill_horizontal = 1,
            f:row {
                f:static_text { title = "Export preset:", width = LrView.share("out_label") },
                f:edit_field { value = bind("preset_path"), width_in_chars = 44 },
                f:push_button { title = "Choose...", action = choose_preset },
            },
            f:row {
                f:static_text { title = "Folder:", width = LrView.share("out_label") },
                f:edit_field { value = bind("folder"), width_in_chars = 44 },
                f:push_button { title = "Choose...", action = choose_folder },
            },
            f:checkbox { title = "Export the blank reference (<photo>.jpg)", value = bind("export_blank") },
            f:checkbox { title = "Replace files that already exist", value = bind("overwrite") },
        },
    }

    local result = LrDialogs.presentModalDialog {
        title = "Export Reference JPEGs",
        contents = contents,
        actionVerb = "Export",
    }
    if result ~= "ok" then return end

    local preset, problem = read_preset(props.preset_path or "")
    if not preset then
        LrDialogs.message(PLUGIN, "Export preset: " .. tostring(problem), "critical")
        return
    end
    if (props.folder or "") == "" then
        props.folder = preset_folder(preset) or ""
        if props.folder == "" then
            LrDialogs.message(PLUGIN, "Choose a folder for the reference JPEGs.", "critical")
            return
        end
    end

    local chosen = {}
    for _, slider in ipairs(SLIDERS) do
        if props[slider.id .. "_on"] then
            local parse = slider.curves and parse_curve_names or parse_values
            local parsed, bad = parse(props[slider.id .. "_values"])
            if parsed == nil then
                LrDialogs.message(PLUGIN, slider.label .. ": " .. bad, "critical")
                return
            end
            chosen[#chosen + 1] = { id = slider.id, suffix = slider.suffix, relative = slider.relative, parsed = parsed, curves = slider.curves, colours = slider.colours }
        end
    end
    if #chosen == 0 and not props.export_blank then
        LrDialogs.message(PLUGIN, "Nothing to export: tick at least one slider.", "warning")
        return
    end

    prefs.preset_path = props.preset_path
    prefs.folder = props.folder
    prefs.export_blank = props.export_blank
    prefs.overwrite = props.overwrite
    for _, slider in ipairs(SLIDERS) do
        prefs[slider.id .. "_on"] = props[slider.id .. "_on"]
        prefs[slider.id .. "_values"] = props[slider.id .. "_values"]
    end

    run(context, props, chosen, preset)
end

LrTasks.startAsyncTask(function()
    LrFunctionContext.callWithContext("DevelopReferences", function(context)
        LrDialogs.attachErrorDialogToFunctionContext(context)
        dialog(context)
    end)
end)
