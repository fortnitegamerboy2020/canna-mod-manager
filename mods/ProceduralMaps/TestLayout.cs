using System;
using Canna.ProceduralMaps;
class TestLayout
{
    static void Main()
    {
        for (uint seed = 0; seed < 10000; seed++)
        {
            Layout a = Layout.Generate(seed, 6 + (int)(seed % 11));
            Layout b = Layout.Generate(seed, a.islands.Length);
            if (a.fingerprint != b.fingerprint) throw new Exception("Client layouts diverged");
            for (int i = 0; i < a.islands.Length; i++)
            {
                Island p = a.islands[i];
                for (int j = i + 1; j < a.islands.Length; j++)
                {
                    Island q = a.islands[j];
                    bool separateX = Math.Abs(p.x - q.x) > p.width + q.width + p.radius + q.radius + p.drift + q.drift;
                    bool separateY = Math.Abs(p.y - q.y) > p.height + q.height + p.radius + q.radius + 200;
                    if (!separateX && !separateY) throw new Exception("Islands can overlap");
                }
                for (int tick = 0; tick < p.period * 2; tick++)
                    if (Math.Abs(Layout.DriftAt(p, tick)) > p.drift) throw new Exception("Movement escaped bounds");
                if (Layout.DriftAt(p, 0) != Layout.DriftAt(p, p.period)) throw new Exception("Motion not periodic");
                if (i < 4 && p.drift != 0) throw new Exception("Starting island moves");
            }
        }
        Console.WriteLine("10,000 seeds passed: identical client layouts, safe spawn islands, separated platforms and bounded periodic movement.");
    }
}
