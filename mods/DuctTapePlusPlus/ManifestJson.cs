// MIT. Decode only the fixed compatibility schema using the game's Newtonsoft.Json.
// No automatic CLR type construction, Unity serialization or raw error text is used.
using System;
using System.IO;
using System.Linq;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

namespace Canna.DuctTapePlusPlus
{
    public sealed class ManifestDecodeException : Exception
    {
        public readonly string Reason;
        internal ManifestDecodeException(string reason) : base(reason) { Reason = reason; }
    }

    public static class ManifestJson
    {
        public const int MaximumTextLength = 8 * 1024 * 1024;

        sealed class ConsentReader : JsonTextReader
        {
            internal ConsentReader(string text) : base(new StringReader(text)) {
                MaxDepth = 8; DateParseHandling = DateParseHandling.None; SupportMultipleContent = true;
            }
            public override bool Read() {
                bool result = base.Read();
                if (!result) return false;
                if (TokenType == JsonToken.Comment || TokenType == JsonToken.StartConstructor || TokenType == JsonToken.EndConstructor || TokenType == JsonToken.Undefined ||
                    ((TokenType == JsonToken.String || TokenType == JsonToken.PropertyName) && QuoteChar != '"'))
                    throw new JsonReaderException("Invalid consent JSON");
                return true;
            }
        }

        // Consent is read only from the desktop's settings, never from a room peer.
        // Missing, corrupt, duplicate or non-boolean preferences always mean off.
        public static bool ReportingConsent(string text)
        {
            if (System.String.IsNullOrEmpty(text) || text.Length > 65536) return false;
            try {
                bool quoted = false, escaped = false;
                for (int i = 0; i < text.Length; i++) {
                    char c = text[i];
                    if (quoted) { if (escaped) escaped = false; else if (c == '\\') escaped = true; else if (c == '"') quoted = false; continue; }
                    if (c == '"') quoted = true;
                    else if (c == ',') { int next = i + 1; while (next < text.Length && Whitespace(text[next])) next++; if (next < text.Length && (text[next] == '}' || text[next] == ']')) return false; }
                }
                using (var reader = new ConsentReader(text)) {
                    var root = JObject.Load(reader, new JsonLoadSettings {
                        DuplicatePropertyNameHandling = DuplicatePropertyNameHandling.Error,
                        CommentHandling = CommentHandling.Load,
                        LineInfoHandling = LineInfoHandling.Ignore
                    });
                    if (reader.Read()) return false;
                    var consent = root["anonymous_reports"];
                    return consent != null && consent.Type == JTokenType.Boolean && consent.Value<bool>();
                }
            } catch { return false; }
        }
        public static bool ReadReportingConsent(string path)
        {
            try {
                using (var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite))
                using (var reader = new StreamReader(file)) {
                    var chars = new char[65537];
                    int count = reader.ReadBlock(chars, 0, chars.Length);
                    return count <= 65536 && ReportingConsent(new string(chars, 0, count));
                }
            } catch { return false; }
        }

        sealed class BoundedReader : JsonTextReader
        {
            int tokens;
            internal BoundedReader(string text) : base(new StringReader(text))
            {
                MaxDepth = 4;
                DateParseHandling = DateParseHandling.None;
                SupportMultipleContent = true;
            }
            public override bool Read()
            {
                bool result = base.Read();
                if (!result) return false;
                if (++tokens > 200000) throw new ManifestDecodeException("Manifest token count exceeds limits");
                if (TokenType == JsonToken.Comment || TokenType == JsonToken.StartConstructor || TokenType == JsonToken.EndConstructor)
                    throw new ManifestDecodeException("Manifest contains unsupported JSON syntax");
                if (TokenType == JsonToken.PropertyName || TokenType == JsonToken.String)
                {
                    var value = Value as string;
                    if (QuoteChar != '"' || value == null || value.Length > (TokenType == JsonToken.PropertyName ? 64 : 2048))
                        throw new ManifestDecodeException("Manifest JSON string syntax or length is invalid");
                    for (int i = 0; i < value.Length; i++)
                    {
                        char c = value[i];
                        if (c < 32 || c == 127) throw new ManifestDecodeException("Manifest strings contain control characters");
                        if (Char.IsHighSurrogate(c))
                        {
                            if (++i >= value.Length || !Char.IsLowSurrogate(value[i]))
                                throw new ManifestDecodeException("Manifest strings contain invalid Unicode");
                        }
                        else if (Char.IsLowSurrogate(c)) throw new ManifestDecodeException("Manifest strings contain invalid Unicode");
                    }
                }
                return true;
            }
        }

        public static CompatibilityManifest Parse(string text)
        {
            if (text == null || text.Length == 0 || text.Length > MaximumTextLength)
                throw new ManifestDecodeException("Manifest text is missing or exceeds limits");
            RequireStrictSyntax(text);
            JObject root;
            try
            {
                using (var reader = new BoundedReader(text))
                {
                    root = JObject.Load(reader, new JsonLoadSettings {
                        DuplicatePropertyNameHandling = DuplicatePropertyNameHandling.Error,
                        CommentHandling = CommentHandling.Load,
                        LineInfoHandling = LineInfoHandling.Ignore
                    });
                    if (reader.Read()) throw new ManifestDecodeException("Manifest contains trailing JSON content");
                }
            }
            catch (JsonException)
            { throw new ManifestDecodeException("Manifest JSON syntax, depth or duplicate properties are invalid"); }
            RequireKeys(root, "protocol", "profile", "game_sha256", "digest", "assemblies", "files");
            var assemblies = Array(root["assemblies"], 4096);
            var files = Array(root["files"], 16384);
            return new CompatibilityManifest {
                protocol = String(root["protocol"]), profile = String(root["profile"]),
                game_sha256 = String(root["game_sha256"]), digest = String(root["digest"]),
                assemblies = assemblies.Select(token => {
                    var row = Object(token); RequireKeys(row, "identity", "sha256");
                    return new AssemblyRow { identity = String(row["identity"]), sha256 = String(row["sha256"]) };
                }).ToArray(),
                files = files.Select(token => {
                    var row = Object(token); RequireKeys(row, "root", "path", "sha256");
                    return new FileRow { root = String(row["root"]), path = String(row["path"]), sha256 = String(row["sha256"]) };
                }).ToArray()
            };
        }

        static JObject Object(JToken token)
        {
            var value = token as JObject;
            if (value == null) throw new ManifestDecodeException("Manifest row must be a JSON object");
            return value;
        }
        static JArray Array(JToken token, int limit)
        {
            var value = token as JArray;
            if (value == null || value.Count > limit) throw new ManifestDecodeException("Manifest array type or count is invalid");
            return value;
        }
        static string String(JToken token)
        {
            if (token == null || token.Type != JTokenType.String)
                throw new ManifestDecodeException("Manifest field must be a JSON string");
            return token.Value<string>();
        }
        static void RequireKeys(JObject row, params string[] keys)
        {
            var names = row.Properties().Select(p => p.Name).ToArray();
            if (names.Length != keys.Length || keys.Any(key => !names.Contains(key, StringComparer.Ordinal)))
                throw new ManifestDecodeException("Manifest object has missing or unknown fields");
        }

        // Json.NET also accepts JavaScript comments, bare names and trailing commas.
        // This schema has only quoted strings, arrays and objects; reject those extensions
        // before using its tested JSON escape/object decoder. This is a bounded lexical pass.
        static void RequireStrictSyntax(string text)
        {
            bool quoted = false, escaped = false;
            for (int i = 0; i < text.Length; i++)
            {
                char c = text[i];
                if (quoted)
                {
                    if (escaped) escaped = false;
                    else if (c == '\\')
                    {
                        if (i + 1 < text.Length && text[i + 1] == 'u')
                        {
                            int code = EscapeCode(text, i + 2);
                            if (code >= 0xd800 && code <= 0xdbff)
                            {
                                if (i + 11 >= text.Length || text[i + 6] != '\\' || text[i + 7] != 'u')
                                    throw new ManifestDecodeException("Manifest strings contain invalid Unicode escapes");
                                int low = EscapeCode(text, i + 8);
                                if (low < 0xdc00 || low > 0xdfff)
                                    throw new ManifestDecodeException("Manifest strings contain invalid Unicode escapes");
                                i += 11;
                            }
                            else
                            {
                                if (code >= 0xdc00 && code <= 0xdfff)
                                    throw new ManifestDecodeException("Manifest strings contain invalid Unicode escapes");
                                i += 5;
                            }
                        }
                        else escaped = true;
                    }
                    else if (c == '"') quoted = false;
                    continue;
                }
                if (c == '"') { quoted = true; continue; }
                if (c == ',' )
                {
                    int next = i + 1;
                    while (next < text.Length && Whitespace(text[next])) next++;
                    if (next < text.Length && (text[next] == '}' || text[next] == ']'))
                        throw new ManifestDecodeException("Manifest JSON contains a trailing comma");
                }
                if (!(Whitespace(c) || c == '{' || c == '}' || c == '[' || c == ']' || c == ':' || c == ','))
                    throw new ManifestDecodeException("Manifest contains unsupported JSON syntax");
            }
        }
        static int EscapeCode(string text, int at)
        {
            if (at + 3 >= text.Length) throw new ManifestDecodeException("Manifest strings contain invalid Unicode escapes");
            int value = 0;
            for (int i = at; i < at + 4; i++)
            {
                char c = text[i];
                int digit = c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : c >= 'A' && c <= 'F' ? c - 'A' + 10 : -1;
                if (digit < 0) throw new ManifestDecodeException("Manifest strings contain invalid Unicode escapes");
                value = value * 16 + digit;
            }
            return value;
        }
        static bool Whitespace(char c) { return c == ' ' || c == '\t' || c == '\r' || c == '\n'; }
    }
}
