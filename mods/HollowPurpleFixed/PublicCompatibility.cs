// Canna public-ROUNDS adapter. Original implementation; Canna MIT license.
// These narrow UnboundLib-shaped interfaces support only HollowPurple's calls.
// They are not a general replacement for UnboundLib or MMHook.
using System;
using System.Collections;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using HarmonyLib;
using UnityEngine;
using Photon.Pun;
using Photon.Realtime;
using ExitGames.Client.Photon;

namespace Canna.HollowPurplePublic
{
    public static class Native
    {
        // Only the upstream opt-in diagnostics call this wrapper. Real card picks
        // keep their native animation and network lifecycle.
        public static void SmokePick(ApplyCardStats card, Player[] players)
        {
            var info = card.GetComponent<CardInfo>();
            if (!info || !info.sourceCard || info.sourceCard.gameObject == card.gameObject)
                throw new InvalidOperationException("Diagnostic card must be a disposable clone.");
            try { card.OFFLINE_Pick(players); }
            finally
            {
                card.gameObject.SetActive(false);
                UnityEngine.Object.Destroy(card.gameObject);
            }
        }
        public static CardCategory Category()
        {
            var category = ScriptableObject.CreateInstance<CardCategory>();
            category.hideFlags = HideFlags.HideAndDontSave;
            return category;
        }
        public static void Damage(Damagable victim, Vector2 damage, Vector2 position,
            GameObject weapon, Player owner, bool lethal, bool ignoreBlock)
        {
            victim.TakeDamage(damage, position, weapon, owner, lethal, ignoreBlock, HealthHandler.DamageSource.Player);
        }
        public static void Rpc(PhotonView view, string method, RpcTarget targets, object[] args)
        {
            if (method == "RPCA_SendTakeDamage" && args.Length == 4)
                args = args.Concat(new object[] { HealthHandler.DamageSource.Player }).ToArray();
            view.RPC(method, targets, args);
        }
        public static IEnumerator CreatePlayer(PlayerAssigner assigner, InControl.InputDevice device, bool ai)
        {
            int before = PlayerManager.instance.players.Count;
            assigner.CreatePlayer(device, ai);
            float deadline = Time.realtimeSinceStartup + 20;
            while (PlayerManager.instance.players.Count <= before && Time.realtimeSinceStartup < deadline)
                yield return new WaitForSecondsRealtime(.1f);
            if (PlayerManager.instance.players.Count <= before)
                throw new InvalidOperationException("Public ROUNDS player creation did not complete.");
        }
    }

    public sealed class Driver : MonoBehaviour
    {
        public static Driver Instance;
        readonly Dictionary<Player, HashSet<CardInfo>> observed = new Dictionary<Player, HashSet<CardInfo>>();
        bool battle;
        bool playing;
        int roomActors = -1;
        public static void Ensure()
        {
            if (Instance) return;
            var root = new GameObject("Canna HollowPurple public compatibility");
            UnityEngine.Object.DontDestroyOnLoad(root);
            Instance = root.AddComponent<Driver>();
            new Harmony("canna.hollowpurple.public.compatibility").PatchAll(typeof(Driver).Assembly);
        }
        void Update()
        {
            UnboundLib.Cards.CustomCard.Flush();
            bool nextPlaying = GameManager.instance && GameManager.instance.isPlaying;
            bool nextBattle = nextPlaying && GameManager.instance.battleOngoing;
            if (battle && !nextBattle) UnboundLib.GameModes.GameModeManager.Fire("PointEnd");
            if (playing && !nextPlaying) UnboundLib.GameModes.GameModeManager.Fire("GameEnd");
            if (!battle && nextBattle) UnboundLib.GameModes.GameModeManager.Fire("BattleStart");
            battle = nextBattle; playing = nextPlaying;
            int actors = PhotonNetwork.InRoom ? PhotonNetwork.CurrentRoom.PlayerCount : -1;
            if (actors != roomActors && actors >= 0) UnboundLib.Unbound.Handshake();
            roomActors = actors;
            if (!PlayerManager.instance) return;
            foreach (var player in PlayerManager.instance.players)
            {
                if (!player || player.data == null) continue;
                HashSet<CardInfo> previous;
                if (!observed.TryGetValue(player, out previous)) previous = new HashSet<CardInfo>();
                var next = new HashSet<CardInfo>(player.data.currentCards.Where(c => c && c.GetComponent<UnboundLib.Cards.CustomCard>()));
                foreach (var card in next.Where(c => !previous.Contains(c)))
                    UnboundLib.Cards.CustomCard.Notify(card, player, false);
                foreach (var card in previous.Where(c => c && !next.Contains(c)))
                    UnboundLib.Cards.CustomCard.Notify(card, player, true);
                observed[player] = next;
            }
            foreach (var stale in observed.Keys.Where(p => !p).ToArray()) observed.Remove(stale);
        }
    }

    [HarmonyPatch(typeof(CardInfo), "get_CardName")]
    static class NamePatch
    {
        static bool Prefix(CardInfo __instance, ref string __result)
        { var card = __instance.GetComponent<UnboundLib.Cards.CustomCard>(); if (!card) return true; __result = card.Title; return false; }
    }
    [HarmonyPatch(typeof(CardInfo), "get_CardDescription")]
    static class DescriptionPatch
    {
        static bool Prefix(CardInfo __instance, ref string __result)
        { var card = __instance.GetComponent<UnboundLib.Cards.CustomCard>(); if (!card) return true; __result = card.Description; return false; }
    }
    [HarmonyPatch(typeof(CardChoice), "GetCardPath")]
    static class PathPatch
    {
        static bool Prefix(GameObject __0, ref string __result)
        { var card = __0.GetComponent<UnboundLib.Cards.CustomCard>(); if (!card) return true; __result = card.PrefabKey; return false; }
    }
    [HarmonyPatch(typeof(ApplyCardStats), "ApplyStats")]
    static class ApplyPatch
    {
        static bool Prefix(ApplyCardStats __instance)
        {
            var custom = __instance.GetComponent<UnboundLib.Cards.CustomCard>();
            if (!custom) return true;
            var player = (Player)AccessTools.Field(typeof(ApplyCardStats), "playerToUpgrade").GetValue(__instance);
            if (!player || player.data == null) throw new InvalidOperationException("Custom card has no native player.");
            player.data.currentCards.Add(custom.CanonicalCard);
            AccessTools.Field(typeof(ApplyCardStats), "done").SetValue(__instance, true);
            player.data.stats.WasUpdated();
            player.data.healthHandler.Revive(true);
            // Driver reconciles all card additions/removals, including external resets.
            return false;
        }
    }
    [HarmonyPatch(typeof(CardChoice), "DoPick")]
    static class PickPatch
    { static void Prefix() { UnboundLib.GameModes.GameModeManager.Fire("PickStart"); } }
}

namespace UnboundLib.Cards
{
    public abstract class CustomCard : MonoBehaviour
    {
        class Request { public Type Type; public Action<CardInfo> Callback; }
        static readonly Queue<Request> pending = new Queue<Request>();
        static readonly Dictionary<string, CardInfo> built = new Dictionary<string, CardInfo>();
        static GameObject root;
        public string PrefabKey;
        public CardInfo CanonicalCard { get { return built[PrefabKey]; } }
        public string Title { get { return GetTitle(); } }
        public string Description { get { return GetDescription(); } }
        protected abstract string GetTitle();
        protected abstract string GetDescription();
        protected abstract CardInfoStat[] GetStats();
        protected abstract CardInfo.Rarity GetRarity();
        protected abstract GameObject GetCardArt();
        protected abstract CardThemeColor.CardThemeColorType GetTheme();
        public abstract string GetModName();
        public virtual void SetupCard(CardInfo card, Gun gun, ApplyCardStats stats, CharacterStatModifiers modifiers) { }
        public virtual void OnAddCard(Player p, Gun gun, GunAmmo ammo, CharacterData data, HealthHandler health, Gravity gravity, Block block, CharacterStatModifiers stats) { }
        public virtual void OnRemoveCard(Player p, Gun gun, GunAmmo ammo, CharacterData data, HealthHandler health, Gravity gravity, Block block, CharacterStatModifiers stats) { }
        public virtual void OnReassignCard(Player p, Gun gun, GunAmmo ammo, CharacterData data, HealthHandler health, Gravity gravity, Block block, CharacterStatModifiers stats) { OnAddCard(p, gun, ammo, data, health, gravity, block, stats); }
        public static void BuildCard<T>(Action<CardInfo> callback) where T : CustomCard
        { Canna.HollowPurplePublic.Driver.Ensure(); pending.Enqueue(new Request { Type = typeof(T), Callback = callback }); Flush(); }
        public static void Flush()
        {
            if (!CardChoice.instance || CardChoice.instance.cards == null || CardChoice.instance.cards.Length == 0) return;
            var pool = PhotonNetwork.PrefabPool;
            if (pool == null || !(bool)AccessTools.Field(typeof(CardChoice), "_cardsPooled").GetValue(null)) return;
            if (!root) { root = new GameObject("Canna HollowPurple card prefabs"); root.SetActive(false); UnityEngine.Object.DontDestroyOnLoad(root); }
            while (pending.Count > 0)
            {
                var request = pending.Peek();
                string key = "Canna_HollowPurple_" + request.Type.Name;
                if (built.ContainsKey(key)) { pending.Dequeue(); request.Callback(built[key]); continue; }
                var template = CardChoice.instance.cards.Where(c => c && !c.GetComponent<CustomCard>())
                    .OrderBy(c => c.GetComponentsInChildren<Component>(true).Length).FirstOrDefault();
                if (!template) return;
                var obj = UnityEngine.Object.Instantiate(template.gameObject, root.transform, false);
                obj.name = key;
                var custom = (CustomCard)obj.AddComponent(request.Type);
                var info = obj.GetComponent<CardInfo>();
                custom.PrefabKey = key;
                info.sourceCard = info;
                info.rarity = custom.GetRarity(); info.colorTheme = custom.GetTheme();
                info.cardStats = custom.GetStats(); info.categories = new CardCategory[0];
                info.blacklistedCategories = new CardCategory[0]; info.allowMultiple = false;
                info.cardArt = custom.GetCardArt();
                if (info.cardArt) info.cardArt.transform.SetParent(root.transform, false);
                AccessTools.Field(typeof(CardInfo), "cardName").SetValue(info, custom.GetTitle());
                AccessTools.Field(typeof(CardInfo), "cardDestription").SetValue(info, custom.GetDescription());
                custom.SetupCard(info, obj.GetComponent<Gun>(), obj.GetComponent<ApplyCardStats>(), obj.GetComponent<CharacterStatModifiers>());
                built.Add(key, info); pool.RegisterPrefab(key, obj);
                CardChoice.instance.cards = CardChoice.instance.cards.Concat(new[] { info }).ToArray();
                pending.Dequeue(); request.Callback(info);
                Debug.Log("Canna HollowPurple public: registered " + request.Type.Name);
                obj.SetActive(true);
            }
        }
        public static void Notify(CardInfo card, Player player, bool removed)
        {
            var custom = card.GetComponent<CustomCard>();
            var gun = player.data.weaponHandler.gun;
            var ammo = gun.GetComponentInChildren<GunAmmo>();
            var gravity = player.GetComponent<Gravity>();
            if (removed) custom.OnRemoveCard(player, gun, ammo, player.data, player.data.healthHandler, gravity, player.data.block, player.data.stats);
            else custom.OnAddCard(player, gun, ammo, player.data, player.data.healthHandler, gravity, player.data.block, player.data.stats);
        }
    }
}

namespace UnboundLib.GameModes
{
    public interface IGameModeHandler { void StartGame(); }
    public static class GameModeManager
    {
        class Handler : IGameModeHandler
        {
            public void StartGame()
            {
                if (!MainMenuHandler.instance) throw new InvalidOperationException("Public ROUNDS menu was not found.");
                if (GM_ArmsRace.instance) GM_ArmsRace.instance.gameObject.SetActive(false);
                // Use the real transition, including intro/menu cleanup and offline setup.
                MainMenuHandler.instance.PlaySandbox();
            }
        }
        static readonly Dictionary<string, List<Func<IGameModeHandler, IEnumerator>>> hooks = new Dictionary<string, List<Func<IGameModeHandler, IEnumerator>>>();
        static readonly Handler handler = new Handler();
        public static string CurrentHandlerID { get { return GM_Test.instance && GM_Test.instance.gameObject.activeInHierarchy ? "Sandbox" : "ArmsRace"; } }
        public static IGameModeHandler CurrentHandler { get { return handler; } }
        public static void SetGameMode(string mode) { if (mode != "Sandbox") throw new NotSupportedException("Only native Sandbox is supported by this adapter."); }
        public static void AddHook(string name, Func<IGameModeHandler, IEnumerator> hook)
        {
            Canna.HollowPurplePublic.Driver.Ensure();
            List<Func<IGameModeHandler, IEnumerator>> list;
            if (!hooks.TryGetValue(name, out list)) { list = new List<Func<IGameModeHandler, IEnumerator>>(); hooks.Add(name, list); }
            if (!list.Contains(hook)) list.Add(hook);
        }
        public static void RemoveHook(string name, Func<IGameModeHandler, IEnumerator> hook)
        { List<Func<IGameModeHandler, IEnumerator>> list; if (hooks.TryGetValue(name, out list)) list.Remove(hook); }
        public static void Fire(string name)
        { List<Func<IGameModeHandler, IEnumerator>> list; if (hooks.TryGetValue(name, out list)) foreach (var hook in list.ToArray()) Canna.HollowPurplePublic.Driver.Instance.StartCoroutine(hook(handler)); }
    }
}

namespace UnboundLib
{
    public static class Unbound
    {
        static readonly Dictionary<string, Action> callbacks = new Dictionary<string, Action>();
        public static void RegisterHandshake(string id, Action action) { callbacks[id] = action; }
        public static void Handshake() { foreach (var callback in callbacks.Values.ToArray()) callback(); }
    }
    public static class NetworkingManager
    {
        public delegate void PhotonEvent(object[] data);
        static readonly Dictionary<string, PhotonEvent> callbacks = new Dictionary<string, PhotonEvent>();
        public static void RegisterEvent(string channel, PhotonEvent callback) { callbacks[channel] = callback; }
        public static void RaiseEvent(string channel, RaiseEventOptions targets, object[] data)
        {
            var packet = new object[data.Length + 1]; packet[0] = channel; Array.Copy(data, 0, packet, 1, data.Length);
            if (!PhotonNetwork.RaiseEvent(151, packet, targets, SendOptions.SendReliable))
                Debug.LogWarning("Canna HollowPurple public: reliable Photon event was not queued.");
        }
    }
}
