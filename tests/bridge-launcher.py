"""Windows-only launcher checks with stub registration scripts; no registry writes."""
from pathlib import Path
import json
import shutil
import subprocess
import uuid

root = Path(__file__).resolve().parents[1]
fixtures = root / 'artifacts' / 'bridge-launcher-tests' / str(uuid.uuid4())

for fails in (False, True):
    fixture = fixtures / ('failed registration' if fails else 'successful registration')
    scripts = fixture / 'scripts'
    scripts.mkdir(parents=True)
    wrapper = scripts / 'repair-browser-bridge.cmd'
    shutil.copyfile(root / 'scripts' / wrapper.name, wrapper)
    (scripts / 'register-brave.ps1').write_text(
        "'Brave' | Out-File (Join-Path $PSScriptRoot '../events.txt')\n"
        + ('exit 7\n' if fails else 'exit 0\n'), encoding='utf-8')
    (scripts / 'register-edge.ps1').write_text(
        "'Edge' | Out-File (Join-Path $PSScriptRoot '../events.txt') -Append\nexit 0\n",
        encoding='utf-8')
    (scripts / 'test-browser-bridge.ps1').write_text(
        "Write-Output '{\"synthetic\":true}'\n", encoding='utf-8')
    result = subprocess.run(f'"{wrapper}"', shell=True, input='\n',
                            text=True, capture_output=True, timeout=30)
    events = (fixture / 'events.txt').read_text(encoding='utf-16')
    report = fixture / 'artifacts' / 'bridge-diag' / 'desktop-install.json'
    if fails:
        assert result.returncode == 1, result.stdout + result.stderr
        assert 'Installation failed' in result.stdout
        assert 'Edge' not in events and not report.exists()
    else:
        assert result.returncode == 0, result.stdout + result.stderr
        assert 'Brave' in events and 'Edge' in events
        assert json.loads(report.read_text()) == {'synthetic': True}
        assert 'Installation finished' in result.stdout

print('PASS: desktop repair launcher handles spaced paths, runs both installers, '
      'writes diagnostics, and stops on registration failure')
