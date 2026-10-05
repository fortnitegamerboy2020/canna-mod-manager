using HarmonyLib;
using UnityEngine;

namespace Canna.Anvil
{
    // The picker owns its border; world-space ready indicators need their own
    // badge. Keep it separate so the same artwork stays transparent in the picker.
    public sealed class AnvilHudBadge : MonoBehaviour
    {
        internal SpriteRenderer Artwork, Fill, Border;
        internal void Initialize(SpriteRenderer artwork)
        {
            Artwork = artwork;
            Fill = CreateLayer("Anvil HUD team fill", Art.HudFill);
            Border = CreateLayer("Anvil HUD team border", Art.HudBorder);
            Color color = artwork.material.HasProperty("_CircleColor")
                ? artwork.material.GetColor("_CircleColor") : Color.white;
            SetColor(color);
            Refresh();
        }
        SpriteRenderer CreateLayer(string name, Sprite sprite)
        {
            GameObject layer = new GameObject(name);
            layer.transform.SetParent(Artwork.transform, false);
            SpriteRenderer renderer = layer.AddComponent<SpriteRenderer>();
            renderer.sprite = sprite;
            renderer.sharedMaterial = Plugin.Steel;
            return renderer;
        }
        internal void SetColor(Color fill)
        {
            Color border = new Color(fill.r * .38f, fill.g * .38f, fill.b * .38f, fill.a);
            foreach (TeamColors palette in Resources.FindObjectsOfTypeAll<TeamColors>())
                foreach (TeamColor team in palette.teamColors)
                    if (Mathf.Abs(team.fill.r-fill.r)+Mathf.Abs(team.fill.g-fill.g)+Mathf.Abs(team.fill.b-fill.b)<.001f)
                    { border = team.border; break; }
            Fill.color = fill;
            Border.color = border;
        }
        internal void Refresh()
        {
            bool active = Artwork != null && Artwork.enabled && Artwork.sprite == Art.Icon;
            Fill.enabled = Border.enabled = active;
            if (!active) return;
            Fill.sortingLayerID = Border.sortingLayerID = Artwork.sortingLayerID;
            Fill.sortingOrder = Border.sortingOrder = Artwork.sortingOrder - 1;
            Color fill = Fill.color, border = Border.color;
            fill.a = border.a = Artwork.color.a;
            Fill.color = fill; Border.color = border;
        }
        void LateUpdate() { Refresh(); }
    }
    [HarmonyPatch(typeof(AbilityReadyIndicator), "SetSprite")]
    static class AnvilHudSprite
    {
        static void Postfix(AbilityReadyIndicator __instance, SpriteRenderer ___spriteRen)
        {
            AnvilHudBadge badge = __instance.GetComponent<AnvilHudBadge>();
            if (badge == null && ___spriteRen.sprite == Art.Icon)
            { badge = __instance.gameObject.AddComponent<AnvilHudBadge>(); badge.Initialize(___spriteRen); }
            if (badge != null) badge.Refresh();
        }
    }
    [HarmonyPatch(typeof(AbilityReadyIndicator), "SetColor")]
    static class AnvilHudColor
    {
        static void Postfix(AbilityReadyIndicator __instance, Color __0)
        {
            AnvilHudBadge badge = __instance.GetComponent<AnvilHudBadge>();
            if (badge != null) badge.SetColor(__0);
        }
    }
}
