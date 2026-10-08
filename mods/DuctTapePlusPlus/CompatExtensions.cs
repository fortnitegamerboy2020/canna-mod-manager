// Framework 4.7.2 API bridge for the pinned modern C# scanner sources.
static class CompatExtensions
{
    public static void Deconstruct<TKey, TValue>(this System.Collections.Generic.KeyValuePair<TKey, TValue> pair,
        out TKey key, out TValue value)
    { key = pair.Key; value = pair.Value; }
}
