using System;
using Canna.ProceduralMaps;
class TestLayout
{
    static void Main()
    {
        var kinds=new System.Collections.Generic.HashSet<string>();var counts=new System.Collections.Generic.HashSet<int>();
        int big=0,single=0,maxTop=0;
        for (uint seed = 0; seed < 10000; seed++)
        {
            Layout a = Layout.Generate(seed, 6 + (int)(seed % 11));
            Layout b = Layout.Generate(seed, a.islands.Length);
            kinds.Add(a.kind);counts.Add(a.islands.Length);
            if(a.islands.Length==1)single++;
            if (a.fingerprint != b.fingerprint) throw new Exception("Client layouts diverged");
            for (int i = 0; i < a.islands.Length; i++)
            {
                Island p = a.islands[i];
                if(p.width>=1000)big++;
                maxTop=Math.Max(maxTop,p.y+p.height+p.radius);
                for (int j = i + 1; j < a.islands.Length; j++)
                {
                    Island q = a.islands[j];
                    bool separateX = Math.Abs(p.x - q.x) > p.width + q.width + p.radius + q.radius + p.drift + q.drift + 300;
                    bool separateY = Math.Abs(p.y - q.y) > p.height + q.height + p.radius + q.radius + 300;
                    if (!separateX && !separateY) throw new Exception("Islands can overlap");
                }
                for (int tick = 0; tick < p.period * 2; tick++)
                    if (Math.Abs(Layout.DriftAt(p, tick)) > p.drift) throw new Exception("Movement escaped bounds");
                if (Layout.DriftAt(p, 0) != Layout.DriftAt(p, p.period)) throw new Exception("Motion not periodic");
                if (i < 4 && p.drift != 0) throw new Exception("Starting island moves");
            }
            for(int team=0;team<4;team++)if(a.spawnIslands[team]<0 || a.spawnIslands[team]>=a.islands.Length || a.islands[a.spawnIslands[team]].drift!=0)throw new Exception("Unsafe team spawn island");
            a.FinishNativeGeometry();
            for(int i=0;i<a.islands.Length;i++)for(int j=i+1;j<a.islands.Length;j++)if(!Layout.Separate(a.islands[i],a.islands[j],300))throw new Exception("Native finalization overlaps islands");
        }
        if(kinds.Count!=6 || counts.Count<8 || single<1000 || big<4000)throw new Exception("Insufficient layout diversity");
        Console.WriteLine("10,000 seeds passed: six families, "+counts.Count+" island counts, "+single+" single-island maps, "+big+" large islands; synchronized, separated motion and safe spawns. Highest planned top="+maxTop);
    }
}
