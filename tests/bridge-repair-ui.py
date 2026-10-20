"""Synthetic IPC only: never changes real registry keys, bridge files, or vaults."""
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

MOCK = r"""(() => {
  let id=0;const callbacks={},listeners={};
  window.isTauri=true;
  window.__qa={unlocked:true,repairs:0,mode:'success',commands:[],emit:(name)=>callbacks[listeners[name]]({event:name,payload:null})};
  window.__TAURI_INTERNALS__={
    transformCallback(fn){callbacks[++id]=fn;return id;},
    invoke:async(cmd,args)=>{
      window.__qa.commands.push(cmd);
      if(cmd==='plugin:event|listen'){listeners[args.event]=args.handler;return 1;}
      if(cmd==='vault_status')return {exists:true,unlocked:window.__qa.unlocked,path:'synthetic',idleMinutes:5};
      if(cmd==='hello_status')return {available:true,enabled:true,reason:null};
      if(cmd==='get_app_settings')return {startWithWindows:false,closeToTray:true,startMinimized:false};
      if(cmd==='browser_settings')return {showAccountsWhenLocked:true};
      if(cmd==='list_entries')return [];
      if(cmd==='browser_pending')return null;
      if(cmd==='repair_browser_bridge'){
        window.__qa.repairs++;
        if(args && Object.keys(args).length)throw 'Repair received unexpected frontend arguments';
        await new Promise(resolve=>window.__qa.finish=resolve);
        if(window.__qa.mode==='failure')throw 'Synthetic host is in use; close the extension popup and retry.';
        return {browsers:['Brave','Edge'],restartRequired:true};
      }
      return null;
    }
  };
})();"""

with sync_playwright() as p:
    browser=p.chromium.launch(headless=True)
    page=browser.new_page(viewport={'width':1180,'height':780})
    errors=[]
    page.on('pageerror',lambda e:errors.append(str(e)))
    page.add_init_script(MOCK)
    page.goto('http://127.0.0.1:1420')
    page.wait_for_load_state('networkidle')
    page.locator('#nav-settings').click()
    repair=page.locator('#repair-browser-bridge')
    expect(repair).to_be_visible()
    out=Path('artifacts/bridge-repair')
    out.mkdir(parents=True,exist_ok=True)
    page.screenshot(path=str(out/'settings.png'),full_page=True)
    repair.click()
    expect(repair).to_be_disabled()
    expect(repair).to_have_attribute('aria-busy','true')
    repair.dispatch_event('click')
    assert page.evaluate('window.__qa.repairs')==1
    page.evaluate("window.__qa.unlocked=false;window.__qa.emit('vault-locked')")
    locked=page.locator('#repair-browser-bridge-locked')
    expect(locked).to_be_visible()
    expect(locked).to_be_disabled()
    page.evaluate('window.__qa.finish()')
    expect(page.locator('[data-bridge-repair-status]')).to_contain_text('Bridge repaired for Brave and Edge.')
    expect(locked).to_be_enabled()
    expect(locked).to_have_attribute('aria-busy','false')
    page.evaluate("window.__qa.mode='failure'")
    locked.click()
    expect(locked).to_be_disabled()
    page.evaluate('window.__qa.finish()')
    expect(page.locator('[data-bridge-repair-status]')).to_contain_text('Synthetic host is in use')
    expect(locked).to_be_enabled()
    assert page.evaluate('window.__qa.repairs')==2
    assert not page.evaluate("window.__qa.commands.some(c=>['unlock_hello','unlock_vault','reveal_password','edit_entry'].includes(c))")
    page.screenshot(path=str(out/'locked.png'),full_page=True)
    assert not errors,errors
    browser.close()
print('PASS: Settings and locked repair, no path arguments, duplicate prevention, busy status, lock-during-repair result, failure and retry, no vault access')
