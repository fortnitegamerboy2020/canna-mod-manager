using System;
using System.Text;

namespace Canna.ProceduralMaps
{
    [Serializable]
    public sealed class Island
    {
        // All dimensions are integer hundredths of a game unit.
        public int x, y, width, height, radius, drift, period, phase, spin;
    }
    [Serializable]
    public sealed class Layout
    {
        public uint seed;
        public bool moon;
        public Island[] islands;
        public string fingerprint;
        public string kind;
        public int[] spawnIslands;
        private uint state;
        private int Next(int min, int max)
        {
            state ^= state << 13; state ^= state >> 17; state ^= state << 5;
            return min + (int)(state % (uint)(max - min));
        }
        public static Layout Generate(uint seed, int count)
        {
            Layout map = new Layout();
            map.seed = seed; map.state = seed == 0 ? 0xCABB1234u : seed;
            map.moon = map.Next(0, 3) == 0;
            int family=map.Next(0,6);
            map.kind=new string[]{"continent","canopy","twins","archipelago","stairway","mixed"}[family];
            count=family==0?1:family==1?3:family==2?map.Next(2,5):family==3?map.Next(6,10):family==4?map.Next(3,7):map.Next(4,7);
            var packed=new System.Collections.Generic.List<Island>();
            for (int i = 0; i < count; i++)
            {
                Island p = new Island();
                p.radius=40;p.period=map.Next(480,841);p.phase=map.Next(0,p.period);
                bool big=(family<=1 && i==0) || (family==2 && i<2) || (family==5 && i==0);
                p.width=big ? (family==2?map.Next(800,1001):map.Next(1300,1601)) : map.Next(270,501);
                // Conservative round bounds: native templates can be circular.
                p.height=p.width;
                p.drift=i<4 || big?0:map.Next(0,151);
                if(big)
                {
                    p.x=family==2?(i==0?-1500:1500)+map.Next(-100,101):map.Next(-300,301);
                    p.y=map.Next(-450,-199);
                }
                else
                {
                    bool fits=false;
                    for(int attempt=0;attempt<600;attempt++){
                        p.x=map.Next(-2700,2701);
                        p.y=family==4 ? -300+i*650+map.Next(-150,151) : map.Next(-450,2201);
                        if(family==1)p.x=(i==1?-900:900)+map.Next(-150,151);
                        if(family==1)p.y=packed[0].y+packed[0].height+packed[0].radius+p.height+p.radius+map.Next(350,501);
                        fits=true;foreach(Island q in packed)if(!Separate(p,q,300)){fits=false;break;}
                        if(fits)break;
                        if(attempt==250){p.width=Math.Max(250,p.width*3/4);p.height=p.width;}
                    }
                    // Reject crowded additions rather than build an unreachable tower.
                    if(!fits)continue;
                }
                packed.Add(p);
            }
            map.islands=packed.ToArray();map.spawnIslands=new int[4];
            count=map.islands.Length;
            for(int i=0;i<4;i++){map.spawnIslands[i]=family<=1?0:family==2?i%2:i%Math.Min(4,count);map.islands[map.spawnIslands[i]].drift=0;}
            map.RefreshFingerprint();
            return map;
        }
        public static int NativeScene(uint seed){return Generate(seed,6).moon?((seed & 1)==0?39:41):6+(int)((seed>>16)%33);}
        public static bool Separate(Island p,Island q,int gap){
            return Math.Abs(p.x-q.x)>EnvelopeX(p)+EnvelopeX(q)+p.drift+q.drift+gap
                || Math.Abs(p.y-q.y)>EnvelopeY(p)+EnvelopeY(q)+gap;
        }
        static int EnvelopeX(Island p){return p.spin==0?p.width+p.radius:IntSqrt((long)p.width*p.width+(long)p.height*p.height)+p.radius+1;}
        static int EnvelopeY(Island p){return p.spin==0?p.height+p.radius:EnvelopeX(p);}
        public static int IntSqrt(long n){
            if(n<=0)return 0;long lo=0,hi=Math.Min(n,1000000);
            while(lo<hi){long mid=(lo+hi+1)/2;if(mid*mid<=n)lo=mid;else hi=mid-1;}
            return (int)lo;
        }
        public void FinishNativeGeometry(){
            // Lower satellites after choosing native silhouettes, making the jumps
            // usable above a wide continent as well as above a round moon.
            if(kind=="canopy"){
                Island main=islands[0];int top=main.y+main.height+main.radius;
                int height=Math.Max(islands[1].height+islands[1].radius,islands[2].height+islands[2].radius);
                for(int i=1;i<3;i++)islands[i].y=top+height+450;
            }
            // Native geometry may differ by rounding; keep complete motion envelopes apart.
            for(int i=1;i<islands.Length;i++){
                for(int pass=0;pass<islands.Length;pass++){
                    bool changed=false;
                    for(int j=0;j<i;j++)if(!Separate(islands[i],islands[j],300)){
                        islands[i].y=islands[j].y+EnvelopeY(islands[j])+EnvelopeY(islands[i])+350;changed=true;
                    }
                    if(!changed)break;
                }
            }
        }
        public void RefreshFingerprint()
        {
            uint hash = 2166136261u;
            foreach (byte b in Encoding.UTF8.GetBytes(ToJson())) { hash ^= b; hash = unchecked(hash * 16777619u); }
            fingerprint = hash.ToString("x8");
        }        public string ToJson()
        {
            StringBuilder text = new StringBuilder();
            text.Append("{\"seed\":").Append(seed.ToString(System.Globalization.CultureInfo.InvariantCulture)).Append(",\"kind\":\"").Append(kind).Append("\",\"moon\":").Append(moon ? "true" : "false").Append(",\"islands\":[");
            for (int i = 0; i < islands.Length; i++)
            {
                if (i > 0) text.Append(',');
                Island p = islands[i];
                text.AppendFormat(System.Globalization.CultureInfo.InvariantCulture, "{{\"x\":{0},\"y\":{1},\"width\":{2},\"height\":{3},\"radius\":{4},\"drift\":{5},\"period\":{6},\"phase\":{7},\"spin\":{8}}}", p.x, p.y, p.width, p.height, p.radius, p.drift, p.period, p.phase,p.spin);
            }
            text.Append("]}"); return text.ToString();
        }
        public static int DriftAt(Island p, int tick)
        {
            int t = (int)(((long)tick + p.phase) % p.period);
            if (t < 0) t += p.period;
            int half = p.period / 2;
            return t <= half ? -p.drift + 2 * p.drift * t / half : p.drift - 2 * p.drift * (t - half) / (p.period - half);
        }
    }
}


