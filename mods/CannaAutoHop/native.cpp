// Original Canna native bootstrap, MIT. Interoperability declarations match
// Valve's L4D2 IServerPluginCallbacks002 and VSERVERTOOLS001 public interfaces:
// https://github.com/alliedmodders/hl2sdk/tree/l4d2/public
#include <windows.h>
#include <shellapi.h>
#include <cstring>

using Factory = void* (__cdecl *)(const char*, int*);
struct Vector;
// This prefix includes IBaseInterface's destructor and all overloads preceding
// entity spawning. Unsupported interface versions are rejected during Load.
class ServerTools {
public:
    virtual ~ServerTools() {}
    virtual void* GetIServerEntity(void*) = 0;
    virtual bool SnapPlayerToPosition(const Vector&, const Vector&, void*) = 0;
    virtual bool GetPlayerPosition(Vector&, Vector&, void*) = 0;
    virtual bool SetPlayerFOV(int, void*) = 0;
    virtual int GetPlayerFOV(void*) = 0;
    virtual bool IsInNoClipMode(void*) = 0;
    virtual void* FirstEntity() = 0;
    virtual void* NextEntity(void*) = 0;
    virtual void* FindEntityByHammerID(int) = 0;
    virtual bool GetKeyValue(void*, const char*, char*, int) = 0;
    virtual bool SetKeyValue(void*, const char*, const char*) = 0;
    virtual bool SetKeyValue(void*, const char*, float) = 0;
    virtual bool SetKeyValue(void*, const char*, const Vector&) = 0;
    virtual void* CreateEntityByName(const char*) = 0;
    virtual void DispatchSpawn(void*) = 0;
};

class AutoHop {
    ServerTools* tools = nullptr;
    bool spawnPending = false;
    bool paused = false;
public:
    virtual bool Load(Factory, Factory game) {
        int argc = 0;
        auto argv = CommandLineToArgvW(GetCommandLineW(), &argc);
        bool insecure = false, dedicated = false;
        for (int i = 1; argv && i < argc; ++i) {
            insecure |= _wcsicmp(argv[i], L"-insecure") == 0;
            dedicated |= _wcsicmp(argv[i], L"-dedicated") == 0;
        }
        if (argv) LocalFree(argv);
        if (!insecure || dedicated || !game) return false;
        tools = static_cast<ServerTools*>(game("VSERVERTOOLS001", nullptr));
        return tools != nullptr;
    }
    virtual void Unload() { tools = nullptr; spawnPending = false; }
    virtual void Pause() { paused = true; }
    virtual void UnPause() { paused = false; }
    virtual const char* GetPluginDescription() { return "Canna Auto-Hop 0.2.0 preview (local -insecure)"; }
    virtual void LevelInit(const char*) { spawnPending = false; }
    virtual void ServerActivate(void*, int, int) { spawnPending = true; }
    virtual void GameFrame(bool simulating) {
        if (!simulating || paused || !spawnPending || !tools) return;
        spawnPending = false;
        auto entity = tools->CreateEntityByName("logic_script");
        if (!entity) return;
        tools->SetKeyValue(entity, "targetname", "canna_autohop_bootstrap");
        tools->SetKeyValue(entity, "vscripts", "canna_autohop.nut");
        tools->DispatchSpawn(entity);
    }
    virtual void LevelShutdown() { spawnPending = false; }
    virtual void ClientActive(void*) {}
    virtual void ClientDisconnect(void*) {}
    virtual void ClientPutInServer(void*, const char*) {}
    virtual void SetCommandClient(int) {}
    virtual void ClientSettingsChanged(void*) {}
    virtual int ClientConnect(bool*, void*, const char*, const char*, char*, int) { return 0; }
    virtual int ClientCommand(void*, const void*) { return 0; }
    virtual int NetworkIDValidated(const char*, const char*) { return 0; }
    virtual void OnQueryCvarValueFinished(int, void*, int, const char*, const char*) {}
};
static AutoHop plugin;
extern "C" __declspec(dllexport) void* __cdecl CreateInterface(const char* name, int* result) {
    bool supported = name && std::strcmp(name, "ISERVERPLUGINCALLBACKS002") == 0;
    if (result) *result = supported ? 0 : 1;
    return supported ? &plugin : nullptr;
}
