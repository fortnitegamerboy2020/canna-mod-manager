using UnityEngine;
namespace Canna.Anvil
{
    // Original vector artwork, rasterized locally. No redistributed game textures.
    internal static class Art
    {
        internal static Sprite[] Frames;
        internal static Sprite Icon;
        static readonly Vector2[] Outline = new Vector2[] {
            new Vector2(22,88),new Vector2(96,88),new Vector2(105,82),
            new Vector2(103,72),new Vector2(82,64),new Vector2(78,51),
            new Vector2(86,34),new Vector2(93,30),new Vector2(93,24),
            new Vector2(39,24),new Vector2(39,30),new Vector2(48,36),
            new Vector2(52,51),new Vector2(46,64),new Vector2(31,68),
            new Vector2(13,81)
        };
        internal static void Create()
        {
            if (Frames != null) return;
            Frames = new Sprite[17];
            for (int f=0;f<=16;f++)
            {
                float t=f/16f; t=t*t*(3-2*t);
                Vector2[] polygon=new Vector2[Outline.Length];
                for(int i=0;i<polygon.Length;i++)
                {
                    Vector2 direction=(Outline[i]-new Vector2(64,60)).normalized;
                    polygon[i]=Vector2.Lerp(new Vector2(64,60)+direction*37,Outline[i],t);
                }
                Texture2D texture=new Texture2D(128,128,TextureFormat.RGBA32,false);
                texture.name="Canna Anvil morph "+f;
                Color[] pixels=new Color[128*128];
                for(int y=0;y<128;y++)for(int x=0;x<128;x++)
                {
                    Color color=Color.clear;
                    for(int sy=0;sy<2;sy++)for(int sx=0;sx<2;sx++)
                    {
                        Vector2 p=new Vector2(x+(sx+.5f)/2,y+(sy+.5f)/2);
                        bool inside=Inside(p,polygon); float distance=Distance(p,polygon);
                        Color sample=Color.clear;
                        if(inside || distance<1.6f)
                        {
                            sample=new Color(.15f,.18f,.23f,1);
                            if(inside && distance>2.2f)
                            {
                                float light=Mathf.Lerp(.63f,.86f,Mathf.Clamp01((p.y-25)/65));
                                if(p.y>80 && t>.6f)light=.94f;
                                sample=Color.Lerp(Color.white,new Color(light*.88f,light*.94f,light,1),t);
                                if(t<.65f && (Ellipse(p,57,68,7,10)||Ellipse(p,75,68,7,10)))sample=Color.Lerp(sample,Color.white,1-t/.65f);
                                if(t<.65f && (Ellipse(p,59,67,3,5)||Ellipse(p,77,67,3,5)))sample=Color.Lerp(sample,new Color(.12f,.14f,.18f,1),1-t/.65f);
                                if(p.y>83 && p.y<85 && p.x>32 && p.x<93 && t>.6f)sample=Color.white;
                            }
                        }
                        color+=sample*.25f;
                    }
                    // Store straight alpha; averaging transparent edge samples otherwise darkens them twice.
                    if(color.a>0){color.r/=color.a;color.g/=color.a;color.b/=color.a;}
                    pixels[y*128+x]=color;
                }
                texture.SetPixels(pixels);texture.Apply(false,false);texture.filterMode=FilterMode.Bilinear;
                UnityEngine.Object.DontDestroyOnLoad(texture);
                Frames[f]=Sprite.Create(texture,new Rect(0,0,128,128),new Vector2(.5f,60f/128),24);
                Frames[f].name="Canna Anvil "+f;
                UnityEngine.Object.DontDestroyOnLoad(Frames[f]);
            }
        }
        internal static void CreateIcon(Sprite nativeIcon)
        {
            if(Icon!=null)return;
            // Match the native icon's world diameter: HUD uses SpriteRenderer while
            // the picker uses Image, so a hardcoded UI PPU made the HUD microscopic.
            Texture2D iconTexture=new Texture2D(128,128,TextureFormat.RGBA32,false);
            for(int y=0;y<128;y++)for(int x=0;x<128;x++)
            {
                float sx=(x-64)/.82f+64,sy=(y-64)/.82f+60;
                Color c=sx>=0 && sx<128 && sy>=0 && sy<128 ? Frames[16].texture.GetPixelBilinear(sx/128,sy/128) : Color.clear;
                float distance=Vector2.Distance(new Vector2(x+.5f,y+.5f),new Vector2(64,64));
                Color background=distance<=55?new Color(.32f,.39f,.45f,1):Color.clear;
                if(distance<49)background=new Color(.69f,.78f,.84f,1);
                c=Color.Lerp(background,new Color(c.r,c.g,c.b,1),c.a);
                iconTexture.SetPixel(x,y,c);
            }
            iconTexture.Apply();iconTexture.filterMode=FilterMode.Bilinear;
            float ppu=128f/(nativeIcon.rect.width/nativeIcon.pixelsPerUnit);
            Icon=Sprite.Create(iconTexture,new Rect(0,0,128,128),new Vector2(.5f,.5f),ppu);
            Icon.name="Canna Anvil menu icon";
            UnityEngine.Object.DontDestroyOnLoad(iconTexture);UnityEngine.Object.DontDestroyOnLoad(Icon);
        }
        static bool Ellipse(Vector2 p,float x,float y,float rx,float ry)
        { float a=(p.x-x)/rx,b=(p.y-y)/ry;return a*a+b*b<=1; }
        static bool Inside(Vector2 p,Vector2[] points)
        {
            bool inside=false;
            for(int i=0,j=points.Length-1;i<points.Length;j=i++)
                if((points[i].y>p.y)!=(points[j].y>p.y) && p.x<(points[j].x-points[i].x)*(p.y-points[i].y)/(points[j].y-points[i].y)+points[i].x)inside=!inside;
            return inside;
        }
        static float Distance(Vector2 p,Vector2[] points)
        {
            float d=float.MaxValue;
            for(int i=0;i<points.Length;i++)
            {Vector2 a=points[i],v=points[(i+1)%points.Length]-a;float t=Mathf.Clamp01(Vector2.Dot(p-a,v)/v.sqrMagnitude);d=Mathf.Min(d,(p-a-v*t).magnitude);}
            return d;
        }
    }
}

