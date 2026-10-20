"""Synthetic extension UI checks. Requires a local Vite server and Playwright."""
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

MOCK = r"""(() => {
  window.__qa = {status:'locked',matches:[{id:'synthetic',title:'Example account',username:'synthetic.person@example.test',url:'https://example.test'}],fills:[]};
  window.chrome = {
    tabs:{query:async()=>[{id:42,url:'https://example.test/login'}]},
    runtime:{
      sendNativeMessage:async()=>({version:1,status:window.__qa.status,matches:window.__qa.matches}),
      sendMessage:async(msg)=>{window.__qa.fills.push(msg);return {ok:true};}
    }
  };
})();"""

with sync_playwright() as p:
    browser = p.chromium.launch(headless=True)
    page = browser.new_page(viewport={'width':360,'height':650})
    errors = []
    external = []
    page.on('pageerror', lambda e: errors.append(str(e)))
    page.on('request', lambda r: external.append(r.url) if not r.url.startswith(('http://127.0.0.1:1420','data:')) else None)
    page.add_init_script(MOCK)
    page.goto('http://127.0.0.1:1420/extension/popup.html')
    page.wait_for_load_state('networkidle')
    expect(page.locator('#connection')).to_have_text('Vault locked')
    expect(page.locator('.account-copy > span')).to_have_text('synthetic.person@example.test')
    expect(page.locator('.fill-action > span')).to_have_text('Unlock & fill')
    expect(page.locator('.account-button')).to_have_accessible_name('Unlock and fill Example account, synthetic.person@example.test')
    assert page.evaluate('document.documentElement.scrollWidth<=innerWidth')
    out = Path('artifacts/locked-accounts')
    out.mkdir(parents=True,exist_ok=True)
    page.screenshot(path=str(out/'locked.png'),full_page=True)
    page.locator('.account-button').click()
    expect(page.locator('.fill-action > span')).to_have_text('Unlock & fill')
    assert page.evaluate('window.__qa.fills.length') == 1
    assert page.evaluate('window.__qa.fills[0].id') == 'synthetic'
    page.evaluate("window.__qa.status='ok'")
    page.locator('#refresh').click()
    expect(page.locator('#connection')).to_have_text('Unlocked')
    expect(page.locator('.fill-action > span')).to_have_text('Fill')
    page.locator('.account-button').click()
    expect(page.locator('.fill-action > span')).to_have_text('Fill')
    assert page.evaluate('window.__qa.fills.length') == 2
    page.evaluate("window.__qa.status='locked';window.__qa.matches=[]")
    page.locator('#refresh').click()
    expect(page.locator('#state')).to_contain_text('enable “Show accounts while locked” once')
    expect(page.locator('.account-button')).to_have_count(0)
    assert not errors, errors
    assert not external, external
    browser.close()
print('PASS: locked metadata, Unlock & fill label and handoff, unlocked Fill, repeat clicks, setup guidance, no external requests or overflow')
