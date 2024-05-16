-- usage: lsyncd template.lua and set MACHINES var to a list of machines seperated by :
--        or start-sync <machine> <machine> ...
local pwd = os.getenv("PWD")
local basename = string.match(pwd, "/([^/]+)$")

local env_machines=os.getenv("MACHINES")
if not env_machines then
    print("No machines specfied via env var MACHINES")
    os.exit(1)
end

machines = {}
for machine in env_machines:gmatch("[^:]+") do
    if string.find(machine, "/") then
        print("Be careful with special characters in machine")
        os.exit(1)
    end
    table.insert(machines, "--machine")
    table.insert(machines, machine)
end

settings {
    nodaemon = false,
}

local excludes = {}

function loadExcludes (path)
    local file = io.open(path, "r")
    if file then
        print("Reading ignores from " .. path)
        for line in file:lines() do
            if string.find(line, "noldignore") then
                print("ldignore found. Skipping further entries in " .. path)
                break
            end
            if not(line == "") and not (string.sub(line, 0, 1) == "#") then
                table.insert(excludes, line)
            end
        end
        file:close()
    end
end

-- Add # noldignore as comment in .gitignore to skip following ignore statements
loadExcludes(".gitignore")
loadExcludes(".ldignore")

local mysync = {
    -- we could look into rsyncssh when we need it
    -- the mode uses ssh to move files
    default.rsync,
    -- this code here is needed since lsyncd always adds a trailing
    -- slash to the source
    source = pwd,
    target = basename,
    delay = 0.7,
    -- we might experiment here with delete = "running"
    -- only deletes files deleted while running
    delete = false,
    rsync = {
        binary = "rsync-notify-multiple",
        _extra = machines,
    },
    exclude = excludes
}

sync(mysync)
