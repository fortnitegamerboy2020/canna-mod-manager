using System;
using System.Security.Cryptography;
using HarmonyLib;

namespace Canna.ProceduralMaps
{
    // Select the actual stock scene before it loads, using the same seed that
    // generates the terrain. Online clients already receive this seed in Bopl's
    // native start packet; theme selection must never use a local clock there.
    static class Theme
    {
        internal static uint OfflineSeed=FreshSeed();
        internal static uint Seed { get { return GameLobby.isOnlineGame ? SteamManager.startParameters.seed : OfflineSeed; } }
        static uint FreshSeed() {
            byte[] bytes=new byte[4];using(var rng=new RNGCryptoServiceProvider())rng.GetBytes(bytes);
            return BitConverter.ToUInt32(bytes,0);
        }
        internal static void NewOfflineRound(){if(!GameLobby.isOnlineGame)OfflineSeed=FreshSeed();}
        internal static byte Level(uint seed) {
            // Native base-game space scenes 39 and 41 include both asteroids and
            // robot satellite platforms. No DLC scenes or foreign artwork.
            int scene=Layout.NativeScene(seed);
            return (byte)(scene-6);
        }
    }
    [HarmonyPatch(typeof(GameSessionHandler),"LoadNextLevelScene")]
    static class NewRoundSeed
    {
        static void Prefix(){if(Plugin.Enabled.Value && !GameLobby.isPlayingAReplay)Theme.NewOfflineRound();}
    }
    [HarmonyPatch(typeof(GameSession),"CurrentLevel")]
    static class ChooseNativeTheme
    {
        static void Postfix(ref byte __result){
            if(Plugin.Enabled.Value && !GameLobby.isPlayingAReplay)__result=Theme.Level(Theme.Seed);
        }
    }
}
