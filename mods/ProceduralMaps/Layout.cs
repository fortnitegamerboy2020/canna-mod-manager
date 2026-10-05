using System;
using System.Text;

namespace Canna.ProceduralMaps
{
    [Serializable]
    public sealed class Island
    {
        // All dimensions are integer hundredths of a game unit.
        public int x, y, width, height, radius, drift, period, phase;
    }
    [Serializable]
    public sealed class Layout
    {
        public uint seed;
        public bool moon;
        public Island[] islands;
        public string fingerprint;
        private uint state;
        private int Next(int min, int max)
        {
            state ^= state << 13; state ^= state >> 17; state ^= state << 5;
            return min + (int)(state % (uint)(max - min));
        }
        public static Layout Generate(uint seed, int count)
        {
            if (count < 4 || count > 64) throw new ArgumentOutOfRangeException("count");
            Layout map = new Layout();
            map.seed = seed; map.state = seed == 0 ? 0xCABB1234u : seed;
            map.moon = map.Next(0, 3) == 0;
            map.islands = new Island[count];
            for (int i = 0; i < count; i++)
            {
                Island p = new Island();
                if (i < 4)
                {
                    p.x = -2100 + i * 1400 + map.Next(-80, 81);
                    p.y = -600 + map.Next(-100, 101);
                    p.width = map.Next(230, 331); p.height = map.Next(40, 81);
                    p.drift = 0; // Stable starting islands for all four team spawns.
                }
                else
                {
                    int row = (i - 4) / 4;
                    p.x = -2100 + ((i - 4) % 4) * 1400 + map.Next(-100, 101);
                    p.y = 250 + row * 900 + map.Next(-70, 71);
                    p.width = map.Next(170, 291); p.height = map.Next(30, 71);
                    p.drift = map.Next(50, 121);
                }
                p.radius = 40; p.period = map.Next(480, 841); p.phase = map.Next(0, p.period);
                map.islands[i] = p;
            }
            uint hash = 2166136261u;
            string text = map.ToJson();
            foreach (byte b in Encoding.UTF8.GetBytes(text)) { hash ^= b; hash = unchecked(hash * 16777619u); }
            map.fingerprint = hash.ToString("x8");
            return map;
        }
        public string ToJson()
        {
            StringBuilder text = new StringBuilder();
            text.Append("{\"seed\":").Append(seed.ToString(System.Globalization.CultureInfo.InvariantCulture)).Append(",\"moon\":").Append(moon ? "true" : "false").Append(",\"islands\":[");
            for (int i = 0; i < islands.Length; i++)
            {
                if (i > 0) text.Append(',');
                Island p = islands[i];
                text.AppendFormat(System.Globalization.CultureInfo.InvariantCulture, "{{\"x\":{0},\"y\":{1},\"width\":{2},\"height\":{3},\"radius\":{4},\"drift\":{5},\"period\":{6},\"phase\":{7}}}", p.x, p.y, p.width, p.height, p.radius, p.drift, p.period, p.phase);
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
