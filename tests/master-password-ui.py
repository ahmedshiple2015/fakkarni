"""Run with Playwright installed and `npm run dev` serving localhost:1420.

Disposable browser and synthetic IPC only: never opens the user's vault or PIN UI.
"""
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

MOCK = r"""(() => {
  let unlocked = true, callbackId = 0;
  const callbacks = {}, listeners = {};
  window.isTauri = true;
  window.__qa = {calls:0,mode:'failure',cleared:false};
  window.__TAURI_INTERNALS__ = {
    transformCallback(fn) { callbacks[++callbackId] = fn; return callbackId; },
    invoke: async (cmd,args) => {
      if(cmd==='plugin:event|listen') { listeners[args.event] = args.handler; return 1; }
      if(cmd==='vault_status') return {exists:true,unlocked,path:'synthetic',idleMinutes:5};
      if(cmd==='hello_status') return {available:true,enabled:unlocked,reason:null};
      if(cmd==='get_app_settings') return {startWithWindows:false,closeToTray:true,startMinimized:false};
      if(cmd==='browser_settings') return {showAccountsWhenLocked:false};
      if(cmd==='list_entries') return [];
      if(cmd==='browser_pending') return null;
      if(cmd==='change_master_password') {
        window.__qa.calls++;
        window.__qa.cleared = [...document.querySelectorAll('dialog input')].every(i=>i.value===''&&i.disabled&&i.type==='password');
        await new Promise(resolve=>{window.__qa.finish=resolve;});
        if(window.__qa.mode==='failure') throw 'Synthetic authentication failure';
        unlocked = false;
        callbacks[listeners['vault-locked']]({event:'vault-locked',payload:null});
        await new Promise(resolve=>setTimeout(resolve,50));
        return {recoveryPath:'C:/synthetic/backups/previous.kdbx',warnings:['Synthetic cleanup warning.']};
      }
      return null;
    }
  };
})();"""

with sync_playwright() as p:
    browser = p.chromium.launch(headless=True)
    page = browser.new_page(viewport={'width':1180,'height':800})
    errors = []
    page.on('pageerror', lambda e: errors.append(str(e)))
    page.add_init_script(MOCK)
    page.goto('http://127.0.0.1:1420')
    page.locator('#nav-settings').click()
    page.get_by_role('button',name='Change master password',exact=True).click()
    current = page.locator('#master-change-current')
    new = page.locator('#master-change-new')
    confirm = page.locator('#master-change-confirm')
    form = page.locator('dialog form')
    feedback = page.locator('#master-change-status')
    current.fill('synthetic old master')
    new.fill('synthetic new master')
    confirm.fill('different synthetic value')
    form.dispatch_event('submit')
    expect(feedback).to_contain_text('do not match')
    assert page.evaluate('window.__qa.calls') == 0
    new.fill('a'*11)
    confirm.fill('a'*11)
    form.dispatch_event('submit')
    expect(feedback).to_contain_text('at least 12')
    new.fill('界'*342)
    confirm.fill('界'*342)
    form.dispatch_event('submit')
    expect(feedback).to_contain_text('1,024 bytes')
    assert page.evaluate('window.__qa.calls') == 0
    new.fill('synthetic new master')
    confirm.fill('synthetic new master')
    form.dispatch_event('submit')
    expect(feedback).to_contain_text('Saving')
    assert page.evaluate('window.__qa.cleared')
    page.keyboard.press('Escape')
    expect(page.locator('dialog')).to_be_visible()
    page.evaluate('window.__qa.finish()')
    expect(feedback).to_contain_text('Synthetic authentication failure')
    expect(current).to_be_enabled()
    expect(current).to_have_value('')
    current.fill('synthetic old master')
    new.fill('synthetic new master')
    confirm.fill('synthetic new master')
    page.evaluate("window.__qa.mode='success'")
    form.dispatch_event('submit')
    expect(feedback).to_contain_text('Saving')
    page.evaluate('window.__qa.finish()')
    expect(page.locator('dialog')).to_have_count(0)
    expect(page.locator('body')).to_contain_text('Master password changed.')
    expect(page.locator('body')).to_contain_text('C:/synthetic/backups/previous.kdbx')
    expect(page.locator('body')).to_contain_text('Synthetic cleanup warning.')
    expect(page.locator('#master')).to_be_visible()
    assert not errors, errors
    output = Path('artifacts/master-password-phase1')
    output.mkdir(parents=True,exist_ok=True)
    page.screenshot(path=str(output/'success.png'),full_page=True)
    browser.close()
print('PASS: validation, cleared secrets, blocked cancellation, failed authentication, early lock event, committed success and warnings')
