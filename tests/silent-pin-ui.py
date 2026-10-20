"""Synthetic native-event UI test; never opens a real vault or Windows PIN prompt."""
from playwright.sync_api import sync_playwright, expect

MOCK = r"""(() => {
  let id=0; const callbacks={}, listeners={};
  window.isTauri=true;
  window.__qa={unlocked:false,commands:[],emit:(name)=>callbacks[listeners[name]]({event:name,payload:null})};
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
    expect(page.locator('#hello-unlock')).to_be_visible()
    page.evaluate("window.__qa.unlocked=true;window.__qa.emit('browser-vault-unlocked')")
    expect(page.locator('#vault-panel')).to_be_visible()
    expect(page.locator('#hello-unlock')).to_have_count(0)
    assert not page.evaluate("window.__qa.commands.some(c=>c==='unlock_hello'||c==='unlock_vault'||c==='enable_hello')")
    page.evaluate("window.__qa.unlocked=false;window.__qa.emit('vault-locked')")
    expect(page.locator('#hello-unlock')).to_be_visible()
    assert not errors,errors
    browser.close()
print('PASS: native browser unlock event refreshes hidden UI without another auth command; subsequent lock restores locked state')
