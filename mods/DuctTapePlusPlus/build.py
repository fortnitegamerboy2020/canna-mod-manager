#!/usr/bin/env python3
"""Build the task-local Canna Rebound preview bundle; never install into ROUNDS.

Pinned public downloads are read as data, SHA checked, and compiled in target/.
The original UnboundLib project is never built: it has a game-writing postbuild.
"""
import argparse
import bz2
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent
WORKSPACE = ROOT.parent.parent
DUCT_COMMIT = "02e1b3d561f1a3dee321a893f111934ab3a2e1df"
TOOLKIT_COMMIT = "a20bbb2eb5ab8fd2bf0813da4a7d7af0f3bcde28"
UNBOUND_COMMIT = "000191da4cbd1051f329aa8b553c955400477ce8"
FRIENDS_COMMIT = "8aac89b7e99911752efb86af6593b83f0fd3469d"
PROTOCOL = "canna.ducttape++/1"
LIMIT = 128 * 1024 * 1024


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(command, env=None):
    subprocess.run([str(v) for v in command], check=True, env=env)


def source_tree(path, repository, commit, downloads, owner="KieranK07"):
    path = Path(path) if path else downloads / repository
    if not path.exists():
        run(["git", "clone", "https://github.com/" + owner + "/" + repository + ".git", path])
        run(["git", "-C", path, "checkout", "--detach", commit])
    actual = subprocess.check_output(["git", "-C", str(path), "rev-parse", "HEAD"], text=True).strip()
    if actual != commit:
        raise ValueError(f"Unexpected {repository} commit: {actual}")
    if subprocess.check_output(["git", "-C", str(path), "status", "--porcelain"], text=True).strip():
        raise ValueError(f"Upstream {repository} source is modified; use a clean pinned tree")
    return path.resolve()


def download(url, destination):
    if destination.exists():
        return destination.read_bytes()
    request = urllib.request.Request(url, headers={"User-Agent": "Canna-DuctTapePlusPlus-local-build"})
    with urllib.request.urlopen(request, timeout=90) as response:
        data = response.read(LIMIT + 1)
    if len(data) > LIMIT:
        raise ValueError("Pinned source download exceeds limit")
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(data)
    return data


def off(data):
    value = int.from_bytes(data, "little", signed=False)
    return -(value & ((1 << 63) - 1)) if value & (1 << 63) else value


def patch(old, delta):
    if delta[:8] != b"BSDIFF40":
        raise ValueError("Invalid pinned BSDIFF patch")
    control_length, diff_length, output_length = [off(delta[p:p + 8]) for p in (8, 16, 24)]
    if not (0 <= control_length <= len(delta) and 0 <= diff_length <= len(delta) and 0 <= output_length <= LIMIT):
        raise ValueError("Invalid patch limits")
    control = io.BytesIO(bz2.decompress(delta[32:32 + control_length]))
    diff = io.BytesIO(bz2.decompress(delta[32 + control_length:32 + control_length + diff_length]))
    extra = io.BytesIO(bz2.decompress(delta[32 + control_length + diff_length:]))
    result = bytearray(output_length)
    output_at = old_at = 0
    while output_at < output_length:
        triple = control.read(24)
        if len(triple) != 24:
            raise ValueError("Truncated pinned patch")
        add, copy, seek = [off(triple[p:p + 8]) for p in (0, 8, 16)]
        if add < 0 or copy < 0 or output_at + add + copy > output_length:
            raise ValueError("Invalid patch span")
        block = diff.read(add)
        if len(block) != add:
            raise ValueError("Truncated patch diff")
        for n, value in enumerate(block):
            index = old_at + n
            result[output_at + n] = (value + (old[index] if 0 <= index < len(old) else 0)) & 255
        output_at += add
        old_at += add
        block = extra.read(copy)
        if len(block) != copy:
            raise ValueError("Truncated patch extra")
        result[output_at:output_at + copy] = block
        output_at += copy
        old_at += seek
    return bytes(result)


def patched_library(toolkit, package, version, filename, before, after, downloads):
    owner, name = package.split("-", 1)
    url = f"https://thunderstore.io/package/download/{owner}/{name}/{version}/"
    archive = download(url, downloads / f"{package}-{version}.zip")
    with zipfile.ZipFile(io.BytesIO(archive)) as source:
        matches = [item for item in source.infolist() if Path(item.filename).name == filename and item.file_size <= LIMIT]
        originals = [source.read(item) for item in matches]
    originals = [data for data in originals if sha(data) == before]
    if len(originals) != 1:
        raise ValueError(f"Expected exactly one pinned original {package}/{filename}")
    rows = [line.split("\t") for line in (toolkit / "patches/patches.tsv").read_text().splitlines()]
    row = next(row for row in rows if len(row) >= 4 and row[1] == before and row[2] == after)
    result = patch(originals[0], (toolkit / "patches" / row[3]).read_bytes())
    if sha(result) != after:
        raise ValueError(f"Curated {filename} output hash mismatch")
    return result


def runtime_source(duct, output, bundle):
    """Adapt only the reviewed source copy for the installed older HarmonyX.

    Never edit the pinned checkout. These two prefixes target fixed parameters;
    old HarmonyX supports positional __0/__1, but not the newer __args injection.
    """
    original = duct / "src/Runtime"
    destination = output / "runtime-source"
    destination.mkdir(parents=True, exist_ok=True)
    upstream_names = {file.name for file in original.iterdir() if file.suffix in (".cs", ".csproj")}
    local_names = {"LocalCardPickerFixes.cs"}
    if upstream_names & local_names:
        raise ValueError("Canna runtime source collides with pinned upstream source")
    expected = upstream_names | local_names
    if any(file.name not in expected for file in destination.iterdir() if file.suffix in (".cs", ".csproj")):
        raise ValueError("Unexpected source in task-local runtime adaptation directory")
    records = []
    for name in sorted(upstream_names):
        data = (original / name).read_bytes()
        before = sha(data)
        if name == "UnboundLibFixes.cs":
            text = data.decode("utf-8")
            replacements = [
                ("static bool Prefix(object[] __args) => __args.Length > 1 && __args[1] is CharacterData data && data != null && data.stats != null;",
                 "static bool Prefix(CharacterData __1) => __1 != null && __1.stats != null;"),
                ("static bool Prefix(object[] __args, MethodBase __originalMethod)",
                 "static bool Prefix(object __0, MethodBase __originalMethod)"),
                ("if (__args.Length == 0 || __args[0] == null) return true;", "if (__0 == null) return true;"),
                ("var c = Traverse.Create(__args[0]);", "var c = Traverse.Create(__0);"),
            ]
            for before_text, after_text in replacements:
                if text.count(before_text) != 1:
                    raise ValueError("Unexpected upstream runtime prefix shape: " + before_text)
                text = text.replace(before_text, after_text)
            data = text.encode("utf-8")
            records.append(dict(file=name, upstream_sha256=before, adapted_sha256=sha(data),
                                changes=["UL_HealthBarRespawns_Fix: fixed CharacterData argument1 instead of unsupported __args",
                                         "UL_UpdateNotice_Fix: fixed argument0 while preserving __originalMethod and skip rules"]))
        if name.endswith(".cs") and b"__args" in data:
            raise ValueError("Unreviewed generic Harmony __args injection remains in runtime: " + name)
        (destination / name).write_bytes(data)
    # Canna-owned runtime repair for the exact supported modern game. The old
    # dependency fix alone repairs card spawning; the native application path
    # also needs to resolve a PlayerID rather than assume a dense roster index.
    for name in sorted(local_names):
        data = (ROOT / name).read_bytes()
        if b"__args" in data:
            raise ValueError("Unreviewed generic Harmony injection in Canna runtime source")
        (destination / name).write_bytes(data)
        records.append(dict(file=name, origin="Canna MIT", upstream_sha256=None,
                            adapted_sha256=sha(data),
                            changes=["ApplyCardStats.Pick PLAYER branch resolves actual PlayerID; team behavior retained",
                                     "CardBarHandler.AddCard resolves a verified bar binding while preserving original PlayerID",
                                     "Unbound Rebuild records player-object ownership of rebuilt bar slots"]))
    (bundle / "source/runtime-source-adaptations.json").write_text(
        json.dumps(dict(upstream_commit=DUCT_COMMIT, reason="HarmonyX positional injection and native card picker identity compatibility", files=records),
                   sort_keys=True, indent=2) + "\n", encoding="utf-8")
    with zipfile.ZipFile(bundle / "source/runtime-adapted-source.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        for name in sorted(expected):
            entry = zipfile.ZipInfo(name, (2026, 10, 8, 0, 0, 0))
            entry.external_attr = 0o100644 << 16
            archive.writestr(entry, (destination / name).read_bytes(), compress_type=zipfile.ZIP_DEFLATED)
    return destination / "Runtime.csproj"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dotnet", required=True)
    parser.add_argument("--game", required=True)
    parser.add_argument("--core")
    parser.add_argument("--output", default=str(WORKSPACE / "target/ducttape-plus-plus"))
    parser.add_argument("--upstream-source")
    parser.add_argument("--toolkit-source")
    parser.add_argument("--unbound-source")
    parser.add_argument("--friends-source")
    args = parser.parse_args()
    output = Path(args.output).resolve()
    game = Path(args.game).resolve()
    core = Path(args.core).resolve() if args.core else game / "BepInEx/core"
    if output == game or game in output.parents or output in game.parents:
        raise ValueError("Build output must be separate from the game installation")
    if not (core / "Mono.Cecil.dll").is_file():
        raise ValueError("BepInEx core references required; no game installation is performed")
    downloads = output / "downloads"
    downloads.mkdir(parents=True, exist_ok=True)
    duct = source_tree(args.upstream_source, "DuctTape", DUCT_COMMIT, downloads)
    toolkit = source_tree(args.toolkit_source, "rounds-porting-toolkit", TOOLKIT_COMMIT, downloads)
    unbound = source_tree(args.unbound_source, "UnboundLib", UNBOUND_COMMIT, downloads)
    friends = source_tree(args.friends_source, "RoundsWithFriends", FRIENDS_COMMIT, downloads, owner="Bknibb")
    lock = json.loads((ROOT / "upstream.lock.json").read_text(encoding="utf-8"))
    for entry in lock["files"]:
        path = ROOT / entry["file"]
        if sha(path.read_bytes()) != entry["vendored_sha256"]:
            raise ValueError("Vendored source differs from reviewed lock: " + entry["file"])
    bundle = output / "support"
    for directory in ("helper", "payloads", "runtime", "licenses", "source"):
        (bundle / directory).mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    environment.update({"DOTNET_CLI_HOME": str(output / "dotnet-home"), "NUGET_PACKAGES": str(output / "nuget"),
                        "DOTNET_NOLOGO": "1", "DOTNET_CLI_TELEMETRY_OPTOUT": "1", "DOTNET_GENERATE_ASPNET_CERTIFICATE": "false",
                        "DOTNET_ADD_GLOBAL_TOOLS_TO_PATH": "0"})
    common = [args.dotnet, "build", "-c", "Release", "--nologo", "-v", "minimal"]
    run(common + [ROOT / "Translator.csproj", f"-p:Core={core}", f"-p:BaseIntermediateOutputPath={output / 'obj-translator'}/",
                  "-o", bundle / "helper"], environment)
    adapted_runtime = runtime_source(duct, output, bundle)
    run(common + [adapted_runtime, f"-p:GameDir={game}", f"-p:Managed={game / 'ROUNDS_Data/Managed'}",
                  f"-p:BaseIntermediateOutputPath={output / 'obj-runtime'}/", "-o", output / "runtime-build"], environment)
    shutil.copy2(output / "runtime-build/rounds-port.Runtime.dll", bundle / "runtime/rounds-port.Runtime.dll")
    run(common + [ROOT / "NetworkGuard.csproj", f"-p:GameDir={game}", f"-p:Managed={game / 'ROUNDS_Data/Managed'}",
                  f"-p:Core={core}", f"-p:BaseIntermediateOutputPath={output / 'obj-network'}/", "-o", output / "network-build"], environment)
    shutil.copy2(output / "network-build/Canna.DuctTapePlusPlus.NetworkGuard.dll", bundle / "runtime/Canna.DuctTapePlusPlus.NetworkGuard.dll")
    registry = (ROOT / "vendor/toolkit/old-libraries.tsv").read_text(encoding="utf-8").splitlines()
    for line in (ROOT / "vendor/curated/patches.tsv").read_text(encoding="utf-8").splitlines():
        fields = line.split("\t")
        if len(fields) >= 4 and fields[0] in ("UnboundLib.dll", "MMHOOK_Assembly-CSharp.dll", "RoundsWithFriends.dll"):
            registry.extend(f"{fields[0]}\t{digest}\tknown upstream curated library" for digest in fields[1:3])
    (bundle / "helper/old-libraries.tsv").write_text("\n".join(sorted(set(registry))) + "\n", encoding="utf-8")
    specifications = [
        ("UnboundLib", "willis81808-UnboundLib", "3.2.14", "UnboundLib.dll", "ffdb9d0b6482477cbc94d4866023ac84d4956f6a517fcf40510ad4f52a815b29", "2eecd826d889cc1dda5efc0433bc77ab9bafa85cacc00323f3a8c7de79558154", ["MMHOOK_Assembly-CSharp", "Octokit"], "No declared license; partial author source permission recorded; inherited rights unverified"),
        ("MMHOOK_Assembly-CSharp", "willis81808-MMHook", "1.0.0", "MMHOOK_Assembly-CSharp.dll", "febd1c9c7b7e264638bf83078341e707c45a11971d92ebb51b52fac5f1af2b8d", "926b53b329d94f6a8842e6d51ca17ff96f081df59695d7862845d5ccce9e5a62", [], "No declared license; generated-hook redistribution rights unverified"),
        ("RoundsWithFriends", "olavim-RoundsWithFriends", "2.2.2", "RoundsWithFriends.dll", "d1c6440919412e75ebade7181e509fbd04a34b35ed269ef85b8ead6eb0cfde69", "1bd4d5aa47de0e04661710a77bb0b5f1214dac4b3baabc9364b3418ecbc8ab61", ["UnboundLib"], "GPL-3.0"),
    ]
    payloads = []
    for name, package, version, filename, before, after, dependencies, license_name in specifications:
        data = patched_library(toolkit, package, version, filename, before, after, downloads)
        (bundle / "payloads" / (after + ".dll")).write_bytes(data)
        payloads.append(dict(assembly=name, file=after + ".dll", sha256=after, dependencies=dependencies,
                             declared_aliases=[package + "-" + version], source=package + "-" + version + " -> pinned upstream curated patch", license=license_name))
    # These are the exact upstream hand-made outputs, not arbitrary downloads
    # treated as replacement libraries because a package name happens to match.
    known_outputs = {entry["sha256"]: entry for entry in payloads}
    for line in (toolkit / "patches/patches.tsv").read_text(encoding="utf-8").splitlines():
        fields = line.split("\t")
        if len(fields) < 4 or len(fields) > 4 or not fields[0].endswith(".dll"):
            continue
        package_version, archive_file = fields[0].split("/", 1)
        package, version = package_version.rsplit("-", 1)
        if fields[2] in known_outputs:
            entry = known_outputs[fields[2]]
            if package_version not in entry["declared_aliases"]:
                entry["declared_aliases"].append(package_version)
            continue
        data = patched_library(toolkit, package, version, Path(archive_file).name, fields[1], fields[2], downloads)
        destination = bundle / "payloads" / (fields[2] + ".dll")
        destination.write_bytes(data)
        metadata_environment = os.environ.copy()
        metadata_environment["CANNA_METADATA_FILE"] = str(destination)
        name = subprocess.check_output(["powershell.exe", "-NoProfile", "-Command",
            "([Reflection.AssemblyName]::GetAssemblyName($env:CANNA_METADATA_FILE)).Name"], env=metadata_environment, text=True).strip()
        if any(entry["assembly"] == name for entry in payloads):
            raise ValueError(f"Duplicate pinned payload assembly {name}")
        license_name = "GPL-3.0" if name in ("ModdingUtils", "CardBarPatch", "PerformanceImprovements") else "See upstream patch notices; redistribution rights unverified"
        entry = dict(assembly=name, file=fields[2] + ".dll", sha256=fields[2], dependencies=[],
                     declared_aliases=[package_version], source=package_version + " -> exact curated patch", license=license_name)
        payloads.append(entry)
        known_outputs[fields[2]] = entry
    for name, path, dependencies, license_name in [
        ("Octokit", duct / "octokit/Octokit.dll", [], "MIT"),
        ("Sirenix.Serialization", toolkit / "odin/Sirenix.Serialization.dll", ["Sirenix.Serialization.Config", "Sirenix.Utilities"], "Apache-2.0"),
        ("Sirenix.Serialization.Config", toolkit / "odin/Sirenix.Serialization.Config.dll", ["Sirenix.Utilities"], "Apache-2.0"),
        ("Sirenix.Utilities", toolkit / "odin/Sirenix.Utilities.dll", [], "Apache-2.0"),
    ]:
        data = path.read_bytes()
        digest = sha(data)
        (bundle / "payloads" / (digest + ".dll")).write_bytes(data)
        payloads.append(dict(assembly=name, file=digest + ".dll", sha256=digest, dependencies=dependencies,
                             declared_aliases=[], source=str(path.relative_to(duct if name == "Octokit" else toolkit)), license=license_name))
    # This unmodified upstream dependency was exercised by the toolkit's real
    # sweep and is a hard plugin dependency of Cosmic Rounds. Pin the archive
    # and extracted DLL independently before any mechanical normalization.
    card_archive = download("https://thunderstore.io/package/download/Pykess/CardChoiceSpawnUniqueCardPatch/0.1.10/",
                            downloads / "Pykess-CardChoiceSpawnUniqueCardPatch-0.1.10.zip")
    if sha(card_archive) != "311392c9665de8748d1ff8e61d6dd5663ece94292e33184f6e44ce8b3f838820":
        raise ValueError("Pinned CardChoiceSpawnUniqueCardPatch archive differs from toolkit registry")
    with zipfile.ZipFile(io.BytesIO(card_archive)) as archive:
        card_bytes = archive.read("CardChoiceSpawnUniqueCardPatch.dll")
    card_sha = "1b9e8ef4c1d691d3217671096192b1b5d582edf895ac2043f49443552f8a4590"
    if sha(card_bytes) != card_sha:
        raise ValueError("Pinned CardChoiceSpawnUniqueCardPatch DLL hash mismatch")
    (bundle / "payloads" / (card_sha + ".dll")).write_bytes(card_bytes)
    payloads.append(dict(assembly="CardChoiceSpawnUniqueCardPatch", file=card_sha + ".dll", sha256=card_sha,
                         dependencies=["UnboundLib", "ModdingUtils"], declared_aliases=["Pykess-CardChoiceSpawnUniqueCardPatch-0.1.10"],
                         source="Pykess-CardChoiceSpawnUniqueCardPatch-0.1.10; archive SHA256 311392c9665de8748d1ff8e61d6dd5663ece94292e33184f6e44ce8b3f838820",
                         license="Upstream source https://github.com/Rounds-Modding/CardChoiceSpawnUniqueCardPatch; redistribution rights unverified"))
    index = dict(protocol=PROTOCOL, distribution_status="preview-redistribution-unverified", payloads=payloads)
    pinned_payloads = {entry["assembly"]: entry["sha256"] for entry in lock["payloads"]}
    if {entry["assembly"]: entry["sha256"] for entry in payloads} != pinned_payloads:
        raise ValueError("Replacement source bytes differ from reviewed upstream lock")
    (bundle / "payloads/index.json").write_text(json.dumps(index, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    run([bundle / "helper/Canna.DuctTapePlusPlus.Translate.exe", "--prepare-payloads", "--game", game,
         "--core", core, "--report", output / "payload-normalization-report.json"], environment)
    shutil.copy2(output / "payload-normalization-report.json", bundle / "source/payload-normalization-report.json")
    index = json.loads((bundle / "payloads/index.json").read_text(encoding="utf-8"))
    payload_files = {entry["file"] for entry in index["payloads"]}
    for file in (bundle / "payloads").glob("*.dll"):
        if file.name not in payload_files:
            if file.resolve().parent != (bundle / "payloads").resolve():
                raise ValueError("Obsolete build payload escaped task output")
            file.unlink()
    registry.extend(f"{entry['assembly']}.dll\t{entry['sha256']}\tCanna mechanically normalized pinned library"
                    for entry in index["payloads"] if entry["assembly"] in ("UnboundLib", "MMHOOK_Assembly-CSharp", "RoundsWithFriends"))
    (bundle / "helper/old-libraries.tsv").write_text("\n".join(sorted(set(registry))) + "\n", encoding="utf-8")
    for file in (ROOT / "licenses").iterdir():
        shutil.copy2(file, bundle / "licenses" / file.name)
    for tree, label in [(duct, "DuctTape"), (toolkit, "rounds-porting-toolkit"), (unbound, "UnboundLib"), (friends, "RoundsWithFriends")]:
        archive = subprocess.check_output(["git", "-C", str(tree), "archive", "--format=zip", "HEAD"])
        (bundle / "source" / (label + ".zip")).write_bytes(archive)
    local_sources = [file for file in ROOT.rglob("*") if file.is_file() and not any(part in ("bin", "obj", "__pycache__") for part in file.relative_to(ROOT).parts)]
    with zipfile.ZipFile(bundle / "source/Canna-DuctTapePlusPlus.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        for file in sorted(local_sources):
            entry = zipfile.ZipInfo(file.relative_to(ROOT).as_posix(), (2026, 10, 8, 0, 0, 0))
            entry.external_attr = 0o100644 << 16
            archive.writestr(entry, file.read_bytes(), compress_type=zipfile.ZIP_DEFLATED)
    entries = {file.relative_to(bundle).as_posix(): sha(file.read_bytes()) for file in sorted(bundle.rglob("*")) if file.is_file() and file.name != "support-manifest.json"}
    manifest = dict(name="Canna Rebound", protocol=PROTOCOL, profile="rounds-public-1.1.2", preview=True,
                    target_game_sha256="20451cc7090908cd1d125f75f06584645d25e898ec234de0a0c2f154e2900668",
                    distribution_status=index["distribution_status"], upstream=dict(DuctTape=DUCT_COMMIT, toolkit=TOOLKIT_COMMIT), files=entries)
    (bundle / "support-manifest.json").write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    with zipfile.ZipFile(output / "support.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        for file in sorted(bundle.rglob("*")):
            if file.is_file():
                entry = zipfile.ZipInfo(file.relative_to(bundle).as_posix(), (2026, 10, 8, 0, 0, 0))
                entry.external_attr = 0o100644 << 16
                archive.writestr(entry, file.read_bytes(), compress_type=zipfile.ZIP_DEFLATED)
    print(json.dumps({"support": str(output / "support.zip"), "sha256": sha((output / "support.zip").read_bytes()), "files": len(entries)}))


if __name__ == "__main__":
    main()
