[CmdletBinding()]
param(
    [string]$CacheRoot,
    [string]$Python = 'python',
    [ValidateRange(1, 32)][int]$Jobs = 6,
    [switch]$Offline
)
# Intended public location: scripts/prepare-meeting-apm.ps1. This prepares only
# upstream APM; Cargo compiles the reviewed C ABI separately. No global install.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $CacheRoot) {
    $projectRoot = Split-Path -Parent $PSScriptRoot
    $CacheRoot = Join-Path $projectRoot '.cache/native/webrtc-apm-2.1-msvc-x64-md'
}
$CacheRoot = [IO.Path]::GetFullPath($CacheRoot)
if (-not [Environment]::Is64BitOperatingSystem) { throw 'Requires Windows x64.' }
& $Python -c 'import sys,struct,platform; assert sys.version_info >= (3,12), "Python 3.12+ required"; assert struct.calcsize("P")==8 and platform.machine().lower() in ("amd64","x86_64"), "Windows x64 Python required"'
if ($LASTEXITCODE -ne 0) { throw 'Python 3.12+ is required for safe archive extraction.' }
$downloads = Join-Path $CacheRoot 'downloads'
$tools = Join-Path $CacheRoot 'tools'
$sourceParent = Join-Path $CacheRoot 'source'
$source = Join-Path $sourceParent 'webrtc-audio-processing-2.1'
$build = Join-Path $CacheRoot 'build'
$libDir = Join-Path $CacheRoot 'lib'
$tempDir = Join-Path $CacheRoot 'temp'
$licenses = Join-Path $CacheRoot 'licenses'
New-Item -ItemType Directory -Force -Path $downloads,$tools,$sourceParent,$libDir,$tempDir,$licenses | Out-Null

# Validate the actual configured target and every native object before any
# published cache library changes. Also runs for a reused/no-work build.
$validateBuildCode = @'
import hashlib, json, os, pathlib, re, shutil, struct, sys, tempfile

def require(condition, message):
    if not condition:
        raise RuntimeError(message)

def validate_metadata(build):
    info = build / 'meson-info'
    machines = json.loads((info / 'intro-machines.json').read_text())
    for role in ('host', 'target'):
        machine = machines[role]
        require(machine['system'] == 'windows' and machine['cpu_family'] == 'x86_64'
                and machine['endian'] == 'little', 'Wrong Meson ' + role + ' machine')
    compiler = json.loads((info / 'intro-compilers.json').read_text())['host']['cpp']
    require(compiler['id'] == 'msvc', 'Expected actual MSVC compiler')
    options = {o['name']: o['value'] for o in json.loads((info / 'intro-buildoptions.json').read_text())}
    expected = {'b_vscrt': 'md', 'buildtype': 'release', 'default_library': 'static', 'cpp_std': 'c++20'}
    for key, value in expected.items():
        require(options.get(key) == value, 'Wrong effective Meson option: ' + key)
    return {'machines': {k: machines[k] for k in ('host', 'target')},
            'compiler': compiler, 'effective_options': expected}

def validate_archive(archive):
    data = archive.read_bytes()
    require(data.startswith(b'!<arch>\n'), 'Not a COFF static archive')
    offset, count, directive_count = 8, 0, 0
    while offset < len(data):
        require(offset + 60 <= len(data), 'Truncated archive member header')
        header = data[offset:offset+60]
        require(header[58:60] == b'`\n', 'Bad archive member header')
        name = header[:16].decode('ascii').strip()
        size = int(header[48:58].decode('ascii').strip())
        begin, end = offset + 60, offset + 60 + size
        require(end <= len(data), 'Truncated archive member')
        obj = data[begin:end]
        offset = end + (size & 1)
        if name in ('/', '//'):
            continue
        require(len(obj) >= 20, 'Truncated COFF object: ' + name)
        machine, sections = struct.unpack_from('<HH', obj)
        # BigObj/import-object formats are not emitted by the locked build.
        # Reject unsupported headers rather than infer x64 from the host OS.
        require(machine == 0x8664, 'Wrong COFF object machine: ' + name + ' ' + hex(machine))
        optional_size = struct.unpack_from('<H', obj, 16)[0]
        require(optional_size == 0 and sections > 0 and 20 + sections*40 <= len(obj),
                'Unsupported or truncated COFF section table: ' + name)
        directives = []
        for index in range(sections):
            sec = 20 + index*40
            if obj[sec:sec+8].rstrip(b'\0') == b'.drectve':
                length, pointer = struct.unpack_from('<II', obj, sec+16)
                require(pointer + length <= len(obj), 'Truncated COFF directives: ' + name)
                directives.append(obj[pointer:pointer+length].decode('ascii'))
        require(directives, 'Missing release CRT directives: ' + name)
        text = ' '.join(directives)
        libs = {next(v for v in match if v).upper().removesuffix('.LIB')
                for match in re.findall(r'(?i)[/-]DEFAULTLIB:(?:"([^"]+)"|([^\s]+))', text)}
        forbidden = {'MSVCRTD','LIBCMT','LIBCMTD','MSVCPRTD','LIBCPMT','LIBCPMTD','UCRTD','VCRUNTIMED'}
        require('MSVCRT' in libs and not (libs & forbidden), 'Non-/MD release CRT directives: ' + name)
        runtimes = re.findall(r'(?i)RuntimeLibrary=([^"\s]+)', text)
        require(all(value == 'MD_DynamicRelease' for value in runtimes), 'CRT mismatch directive: ' + name)
        require(not re.search(r'(?i)[/-]NODEFAULTLIB', text), 'Unexpected CRT override: ' + name)
        directive_count += 1
        count += 1
    require(offset == len(data) and count > 0, 'Invalid archive length or empty object archive')
    return {'object_members': count, 'machine': 'AMD64/0x8664',
            'release_md_directive_members': directive_count,
            'archive_sha256': hashlib.sha256(data).hexdigest(), 'archive_bytes': len(data)}

def validate_and_publish(build, archive, destination, report_path):
    report = {'metadata': validate_metadata(build), 'coff': validate_archive(archive)}
    # No destination write occurs until both metadata and real objects pass.
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as f:
            temporary = pathlib.Path(f.name)
        shutil.copyfile(archive, temporary)
        require(hashlib.sha256(temporary.read_bytes()).hexdigest() == report['coff']['archive_sha256'],
                'Archive changed while publishing')
        os.replace(temporary, destination)
        temporary = None
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    report_path.write_text(json.dumps(report, indent=2), encoding='utf-8')
    return report

if __name__ == '__main__':
    result = validate_and_publish(*(pathlib.Path(p) for p in sys.argv[1:]))
    print(json.dumps({'validated_coff': result['coff']}, indent=2))
'@

$assets = @(
    @('webrtc-audio-processing-2.1.tar.gz', 'https://freedesktop.org/software/pulseaudio/webrtc-audio-processing/webrtc-audio-processing-2.1.tar.gz', '35e86b986d02ea15f3d04741a1a5a735ba399bc0fac0ee089c39480e35fc3253'),
    @('abseil-cpp-20240722.0.tar.gz', 'https://github.com/abseil/abseil-cpp/releases/download/20240722.0/abseil-cpp-20240722.0.tar.gz', 'f50e5ac311a81382da7fa75b97310e4b9006474f9560ac46f54a9967f07d4ae3'),
    @('abseil-cpp_20240722.0-3_patch.zip', 'https://wrapdb.mesonbuild.com/v2/abseil-cpp_20240722.0-3/get_patch', '12dd8df1488a314c53e3751abd2750cf233b830651d168b6a9f15e7d0cf71f7b'),
    @('meson-1.10.0-py3-none-any.whl', 'https://files.pythonhosted.org/packages/32/4f/c398c6f06ece1c6c246e008d5dac3824c98f54d3eb3d8014f4910afd6d48/meson-1.10.0-py3-none-any.whl', '4b27aafce281e652dcb437b28007457411245d975c48b5db3a797d3e93ae1585'),
    @('ninja-1.13.0-py3-none-win_amd64.whl', 'https://files.pythonhosted.org/packages/29/45/c0adfbfb0b5895aa18cec400c535b4f7ff3e52536e0403602fc1a23f7de9/ninja-1.13.0-py3-none-win_amd64.whl', 'fb8ee8719f8af47fed145cced4a85f0755dd55d45b2bddaf7431fa89803c5f3e')
)
foreach ($asset in $assets) {
    $dest = Join-Path $downloads $asset[0]
    if (-not (Test-Path -LiteralPath $dest)) {
        if ($Offline) { throw "Missing offline asset: $dest" }
        Invoke-WebRequest -Uri $asset[1] -OutFile $dest
    }
    if ((Get-FileHash -LiteralPath $dest -Algorithm SHA256).Hash -ne $asset[2]) {
        throw "Checksum mismatch: $dest"
    }
}

$oldPythonPath = $env:PYTHONPATH
$oldPath = $env:PATH
$oldTemp = $env:TEMP
$oldTmp = $env:TMP
try {
    $env:TEMP = $tempDir
    $env:TMP = $tempDir
    $env:PYTHONPATH = $tools
    $env:PATH = (Join-Path $tools 'bin') + ';' + $oldPath
    if (-not (Test-Path -LiteralPath (Join-Path $tools 'mesonbuild/mesonmain.py'))) {
        & $Python -m pip install --disable-pip-version-check --no-index --no-deps --no-compile --no-warn-script-location --target $tools (Join-Path $downloads $assets[3][0]) (Join-Path $downloads $assets[4][0])
        if ($LASTEXITCODE -ne 0) { throw 'Local Meson/Ninja installation failed.' }
    }
    & $Python -c 'import mesonbuild.coredata,ninja; assert mesonbuild.coredata.version == "1.10.0"; assert ninja.__version__ == "1.13.0"'
    if ($LASTEXITCODE -ne 0) { throw 'Cached build tool versions differ from the lock.' }
    # Filter=data prevents traversal/unsafe link extraction. On reuse, compare
    # every original tar member before compiling; do not trust a marker alone.
    $extractCode = @'
import hashlib, pathlib, sys, tarfile
archive, parent = map(pathlib.Path, sys.argv[1:])
with tarfile.open(archive) as t:
    root = parent / 'webrtc-audio-processing-2.1'
    if not root.exists():
        t.extractall(parent, filter='data')
    for m in t.getmembers():
        if m.isfile():
            path = parent / m.name
            expected = hashlib.sha256(t.extractfile(m).read()).digest()
            if not path.is_file() or hashlib.sha256(path.read_bytes()).digest() != expected:
                raise RuntimeError('Upstream source mismatch: ' + str(path))
'@
    & $Python -c $extractCode (Join-Path $downloads $assets[0][0]) $sourceParent
    if ($LASTEXITCODE -ne 0) { throw 'APM extraction or source verification failed.' }
    $packageCache = Join-Path $source 'subprojects/packagecache'
    New-Item -ItemType Directory -Force -Path $packageCache | Out-Null
    foreach ($index in 1,2) { Copy-Item -LiteralPath (Join-Path $downloads $assets[$index][0]) -Destination $packageCache -Force }
    $setup = @('-u','-m','mesonbuild.mesonmain','setup',$build,$source,'--vsenv','--buildtype=release','--default-library=static','-Db_vscrt=md','-Dcpp_std=c++20','-Dabseil-cpp:cpp_std=c++20','--wrap-mode=forcefallback')
    if (Test-Path -LiteralPath (Join-Path $build 'meson-private/coredata.dat')) { $setup += '--reconfigure' }
    & $Python @setup
    if ($LASTEXITCODE -ne 0) { throw 'APM Meson configuration failed.' }
    $verifyAbseilCode = @'
import hashlib, pathlib, sys, tarfile, zipfile
archive, patch, parent = map(pathlib.Path, sys.argv[1:])
with zipfile.ZipFile(patch) as z:
    overrides = {m.filename: z.read(m) for m in z.infolist() if not m.is_dir()}
with tarfile.open(archive) as t:
    expected = {m.name: t.extractfile(m).read() for m in t.getmembers() if m.isfile()}
expected.update(overrides)
for name, data in expected.items():
    path = parent / name
    if not path.is_file() or hashlib.sha256(path.read_bytes()).digest() != hashlib.sha256(data).digest():
        raise RuntimeError('Abseil source/build patch mismatch: ' + str(path))
'@
    & $Python -c $verifyAbseilCode (Join-Path $downloads $assets[1][0]) (Join-Path $downloads $assets[2][0]) (Join-Path $source 'subprojects')
    if ($LASTEXITCODE -ne 0) { throw 'Abseil source verification failed.' }
    & $Python -u -m mesonbuild.mesonmain compile -C $build webrtc-audio-processing-2 -j $Jobs
    if ($LASTEXITCODE -ne 0) { throw 'APM MSVC compilation failed.' }
    $builtArchive = Join-Path $build 'webrtc/modules/audio_processing/libwebrtc-audio-processing-2.a'
    if (-not (Test-Path -LiteralPath $builtArchive)) { throw 'Missing APM MSVC archive.' }
    $archive = Join-Path $libDir 'webrtc_apm.lib'
    $validationPath = Join-Path $CacheRoot 'validation.json'
    & $Python -c $validateBuildCode $build $builtArchive $archive $validationPath
    if ($LASTEXITCODE -ne 0) { throw 'Actual APM target/COFF/CRT validation failed; published cache was not replaced.' }
    $validation = Get-Content -LiteralPath $validationPath -Raw | ConvertFrom-Json
    # Static Meson APM archive already contains internal and Abseil objects.
    # Do not combine/link separate Abseil archives a second time.
    $noticePaths = @('COPYING','AUTHORS','webrtc/LICENSE','webrtc/PATENTS','webrtc/third_party/rnnoise/COPYING','webrtc/third_party/pffft/LICENSE','webrtc/modules/third_party/fft/LICENSE','webrtc/common_audio/third_party/ooura/LICENSE','webrtc/common_audio/third_party/spl_sqrt_floor/LICENSE','subprojects/abseil-cpp-20240722.0/LICENSE','subprojects/abseil-cpp-20240722.0/AUTHORS','subprojects/abseil-cpp-20240722.0/LICENSE.build')
    foreach ($notice in $noticePaths) {
        $noticeDest = Join-Path $licenses $notice
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $noticeDest) | Out-Null
        Copy-Item -LiteralPath (Join-Path $source $notice) -Destination $noticeDest -Force
    }
    $assetManifest = foreach ($asset in $assets) { [ordered]@{file=$asset[0];uri=$asset[1];sha256=$asset[2]} }
    $manifest = [ordered]@{
        schema=1; backend='webrtc-audio-processing-2.1'; webrtc_version='M131'; abseil_version='20240722.0';
        target='x86_64-pc-windows-msvc'; crt='MD'; cpp_std='c++20'; meson='1.10.0'; ninja='1.13.0';
        configuration='release/static; upstream source unmodified; fixed Abseil fallback';
        archive='lib/webrtc_apm.lib'; archive_bytes=(Get-Item -LiteralPath $archive).Length;
        archive_sha256=(Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant();
        include='source/webrtc-audio-processing-2.1/webrtc';
        abseil_include='source/webrtc-audio-processing-2.1/subprojects/abseil-cpp-20240722.0';
        link_libraries=@('static=webrtc_apm','winmm'); assets=@($assetManifest);
        compiler=((Get-Content -LiteralPath (Join-Path $build 'meson-info/intro-compilers.json') -Raw | ConvertFrom-Json).host.cpp);
        validated_build=$validation;
        prepared_utc=[DateTime]::UtcNow.ToString('o')
    }
    [IO.File]::WriteAllText((Join-Path $CacheRoot 'manifest.json'), ($manifest | ConvertTo-Json -Depth 5), [Text.UTF8Encoding]::new($false))
    Write-Output "Prepared WebRTC APM: $archive"
    Write-Output "MEETING_APM_DIR=$CacheRoot"
} finally {
    $env:PYTHONPATH = $oldPythonPath
    $env:PATH = $oldPath
    $env:TEMP = $oldTemp
    $env:TMP = $oldTmp
}
