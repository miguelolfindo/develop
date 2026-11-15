-- Develop References: exports Lightroom reference JPEGs, one slider at a time, for
-- fitting the Develop raw processor to Lightroom.
return {
    LrSdkVersion = 10.0,
    LrSdkMinimumVersion = 10.0,
    LrToolkitIdentifier = "app.develop.references",
    LrPluginName = "Develop References",
    LrPluginInfoUrl = "https://github.com/miguelolfindo/develop",

    -- Library > Plug-in Extras
    LrLibraryMenuItems = {
        { title = "Export Reference JPEGs...", file = "ExportReferences.lua" },
    },
    -- File > Plug-in Extras
    LrExportMenuItems = {
        { title = "Export Reference JPEGs...", file = "ExportReferences.lua" },
    },

    VERSION = { major = 1, minor = 1, revision = 0, build = 0 },
}
