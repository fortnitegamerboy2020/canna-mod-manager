// Test-only plugin. Excluded from the published mod archive.
using System;
using System.IO;
using System.Linq;
using BepInEx;
using UnityEngine;

[BepInPlugin("canna.hollowpublic.smokechecks", "Canna public port smoke checks", "0.0.1")]
public sealed class PublicSmokeChecks : BaseUnityPlugin
{
    void OnApplicationQuit()
    {
        var args = Environment.GetCommandLineArgs();
        int at = Array.IndexOf(args, "--hp-smoke");
        if (at < 0 || at + 1 >= args.Length) return;
        var report = Path.Combine(args[at + 1], "report.txt");
        if (!File.Exists(report) || File.ReadAllLines(report).Count(l => l.StartsWith("PASS ")) != 70)
            return; // Do not certify an interrupted or incomplete diagnostic run.
        var cards = CardChoice.instance.cards.Where(c => c && c.gameObject.name.StartsWith("Canna_HollowPurple_")).ToArray();
        var orphanCards = Resources.FindObjectsOfTypeAll<CardInfo>().Where(c =>
            c && c.gameObject.scene.IsValid() && c.gameObject.name.StartsWith("Canna_HollowPurple_") &&
            c.gameObject.name.EndsWith("(Clone)") && !c.transform.IsChildOf(CardChoice.instance.transform)).ToArray();
        File.WriteAllLines(Path.Combine(args[at + 1], "public-adapter-checks.txt"), new[] {
            (cards.Length == 26 ? "PASS " : "FAIL ") + "26 public card prototypes registered",
            (cards.All(c => !c.gameObject.activeInHierarchy && c.cardArt && !c.cardArt.activeInHierarchy &&
                c.cardArt.transform.parent && c.cardArt.transform.parent.name == "Canna HollowPurple card prefabs") ? "PASS " : "FAIL ") +
                "Artwork prototypes remain inactive outside card displays",
            (MainMenuHandler.instance && !MainMenuHandler.instance.isOpen &&
                !MainMenuHandler.instance.transform.GetChild(0).gameObject.activeInHierarchy ? "PASS " : "FAIL ") +
                "Native menu is closed during sandbox gameplay",
            (orphanCards.Length == 0 ? "PASS " : "FAIL ") + "Completed diagnostic card clones are destroyed"
        });
    }
}
