const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const fs = require("node:fs/promises");
const path = require("node:path");
const assert = require("node:assert/strict");
const output = path.resolve("reports/module-ui-2026-11-02/ui-evidence");
const order = ["launcher","buddy","sdk","ai","mindmap","api","node","news","mihomo","cert","clipboard","otp","translate","picky","network","httpServer","rtspServer","imageBase64","env","logViewer","jsonBrowser","markdownReader","serial","wsDebugger","music","favorites","tasks","settings","suno","dbdesigner"];
const button = (page, name) => page.getByTestId("fixture-root").getByRole("button", { name, exact: true }).first();
const has = async (page, text) => { await page.getByText(text, { exact: false }).filter({visible:true}).first().waitFor(); };
const dialog = async page => { await page.getByRole("dialog").first().waitFor(); };
const cases = {
 launcher: async p => { await button(p,"添加项目").click(); await dialog(p); },
 buddy: async p => { await button(p,"新增账号").click(); await has(p,"新增 CodeBuddy CN 账号"); await button(p,"粘贴 Token").click(); await has(p,"Token"); },
 sdk: async p => { await button(p,"语言").click(); const input=p.getByPlaceholder("搜索..."); await input.fill("fixture-no-match"); assert.equal(await input.inputValue(),"fixture-no-match"); },
 ai: async p => { await button(p,"添加 Provider").click(); await has(p,"选择预设供应商"); await p.getByRole("button",{name:"自定义供应商",exact:true}).click(); await p.getByRole("button",{name:"确定",exact:true}).waitFor(); await has(p,"协议端点"); },
 mindmap: async p => { await button(p,"文件夹").click(); await has(p,"新建文件夹"); assert.equal(await p.getByRole("button",{name:"创建",exact:true}).isDisabled(),true); },
 api: async p => { await p.addInitScript(()=> { window.__fixtureFaults={api_init:{error:"fixture init failure"}}; }); await p.reload({waitUntil:"domcontentloaded"}); await has(p,"fixture init failure"); await p.evaluate(()=> { window.__fixtureFaults={}; }); await p.getByRole("alert").getByRole("button",{name:"刷新",exact:true}).click(); await p.waitForFunction(()=>window.__fixtureCalls.some(c=>c.command==="api_list_projects")); await p.getByTitle("新建项目").click(); await dialog(p); },
 node: async p => {
   await p.addInitScript(()=> { const dep={name:"fixture",exists:true,path:"C:/fixture/node.exe",version:"22.0.0",satisfies:true,requirement:""}; window.__fixtureOverrides={npm_list_projects:[{id:"fixture-service",displayName:"隔离服务",repo:"",website:"",icon:"Bot",description:"仅浏览器样例",defaultPort:8088,webPath:"/",nodeRequirement:"22",runtime:"node",packageManager:"npm",buildScript:"",startCmd:[],managed:true,npxPackage:"fixture",npxBin:"fixture",consoleUrlPattern:"",autoOpenConsoleUrl:false}],npm_deps:{git:dep,node:dep,packageManager:dep,allReady:true},npm_status:{id:"fixture-service",displayName:"隔离服务",installed:false,status:"not_installed",port:null,pid:null,initAvailable:false,initialized:true}}; });
   await p.reload({waitUntil:"domcontentloaded"}); await has(p,"隔离服务");
   await p.evaluate(()=>window.__fixtureFaults={npm_install:{delay:150,error:"fixture install denied"}}); await button(p,"安装").click(); await has(p,"fixture install denied"); assert.equal((await p.evaluate(()=>window.__fixtureCalls)).filter(c=>c.command==="npm_install").length,1);
   await p.evaluate(async()=> { await window.__fixtureEmit("npm-log",{projectId:"fixture-service",phase:"stdout",line:"fixture log"}); Object.defineProperty(navigator,"clipboard",{configurable:true,value:{writeText:async()=>{throw new Error("fixture clipboard denied");}}}); });
   await has(p,"fixture log"); await p.getByTitle("复制日志").click(); await has(p,"fixture clipboard denied");
   await p.evaluate(()=>Object.defineProperty(navigator,"clipboard",{configurable:true,value:{writeText:async()=>{}}})); await p.getByTitle("复制日志").click(); await has(p,"已复制");
 },
 news: async p => { await button(p,"配置源").click(); await dialog(p); },
 mihomo: async p => { await p.getByTitle("设置").click(); await dialog(p); },
 cert: async p => { await button(p,"新建域名 / 证书").click(); await has(p,"域名"); assert.ok(await p.getByTestId("fixture-root").locator("input").count()>0); },
 clipboard: async p => { await button(p,"设置").click(); await dialog(p); },
 otp: async p => { await button(p,"添加").click(); await dialog(p); },
 translate: async p => { const field=p.getByTestId("fixture-root").locator("textarea"); await field.fill("隔离翻译输入"); await button(p,"清空").click(); assert.equal(await field.inputValue(),""); assert.equal(await button(p,"翻译").isDisabled(),true); },
 picky: async p => { await button(p,"添加收藏").click(); await dialog(p); },
 network: async p => { await button(p,"暂停采样").click(); await has(p,"开始采样"); },
 httpServer: async p => { await button(p,"启动服务").click(); await p.getByRole("alert").waitFor(); assert.equal((await p.evaluate(()=>window.__fixtureCalls)).some(c=>c.command==="start_http_server"),false); },
 rtspServer: async p => {
   await p.addInitScript(()=>localStorage.setItem("any_version_rtsp_instances_v3","{broken"));
   await p.reload({waitUntil:"domcontentloaded"}); await has(p,"已暂停自动保存");
   assert.equal(await p.evaluate(()=>localStorage.getItem("any_version_rtsp_instances_v3")),"{broken");
   await p.screenshot({path:path.join(output,"rtspServer-corrupt.png"),animations:"disabled"});
   await p.addInitScript(()=>localStorage.removeItem("any_version_rtsp_instances_v3"));
   await p.reload({waitUntil:"domcontentloaded"}); await button(p,"添加实例").click(); await has(p,"RTSP 节点 #2");
 },
 imageBase64: async p => { await button(p,"Base64 转 图片").click(); const field=p.getByTestId("fixture-root").locator("textarea").first(); await field.fill("invalid!"); await has(p,"无效"); },
 env: async p => { await button(p,"备份还原").click(); await has(p,"备份"); },
 logViewer: async p => {
   await p.clock.install();
   await p.evaluate(()=> { window.__logReads=0; window.__logContent="first\nERROR second\n"; window.showOpenFilePicker=async()=>[{getFile:async()=>{window.__logReads++;if(window.__logFail)throw new Error("fixture read denied");return new File([window.__logContent],"fixture.log");}}]; });
   await button(p,"打开文件").click(); await has(p,"2 行");
   await button(p,"自动跟随").click();
   for(const text of ["first\nERROR second\nthird\n","x\n","y\n"]) {
     await p.evaluate(text=>window.__logContent=text,text);
     await p.clock.runFor(3100);
     await p.waitForFunction(text=>document.querySelector("#lv-sizer").innerText.includes(text.trim().split("\n").pop()),text);
   }
   await has(p,"1 行");
   await p.evaluate(()=>window.__logFail=true); await p.clock.runFor(3100); await has(p,"fixture read denied");
   await p.evaluate(()=>window.__logFail=false); await p.clock.runFor(3100); assert.equal(await p.locator("#lv-error").isVisible(),false);
   const reads=await p.evaluate(()=>window.__logReads);
   await p.evaluate(()=>Object.defineProperty(document,"hidden",{configurable:true,value:true})); await p.clock.runFor(3100); assert.equal(await p.evaluate(()=>window.__logReads),reads);
   await p.evaluate(()=>delete document.hidden);
   await button(p,"自动跟随").click();
   await p.evaluate(()=>window.__logContent="stopped\n"); await p.clock.runFor(3100);
   assert.ok(!(await p.locator("#lv-sizer").innerText()).includes("stopped"));
   await p.getByLabel("验收模块").selectOption("2");
   await p.waitForFunction(()=>document.querySelector(".lv-root").getClientRects().length===0);
   await p.evaluate(()=>window.__logContent="resumed\n");
   await p.getByLabel("验收模块").selectOption("19");
   await p.waitForFunction(()=>document.querySelector("#lv-sizer").innerText.includes("resumed"));
   assert.equal(await button(p,"自动跟随").getAttribute("aria-pressed"),"false"); await has(p,"1 行");
 },
 jsonBrowser: async p => {
   const editor=p.getByTestId("fixture-root").locator(".monaco-editor textarea").first(); await editor.waitFor({state:"attached",timeout:60000});
   await p.getByTestId("fixture-root").locator(".monaco-editor .view-lines").first().click(); await p.keyboard.press("Control+a"); await p.keyboard.type('{"fixture":[1,2],"ok":true}', {delay:15});
   await has(p,"fixture");
 },
 markdownReader: async p => { await button(p,"隐藏目录").click(); await has(p,"显示目录"); },
 serial: async p => { await button(p,"模拟设备").click(); await has(p,"规则"); },
 wsDebugger: async p => { await button(p,"TCP").click(); await p.getByPlaceholder("主机，如 127.0.0.1").waitFor(); },
 music: async p => { await button(p,"插件").click(); await has(p,"插件"); },
 favorites: async p => { const input=p.getByPlaceholder("搜索标题 / 简介"); await input.fill("fixture"); await p.waitForFunction(()=>window.__fixtureCalls.some(c=>c.command==="fav_list" && c.args.keyword==="fixture")); },
 tasks: async p => { await button(p,"新建任务").click(); await dialog(p); },
 settings: async p => { await button(p,"检查更新").click(); await has(p,"最新"); },
 suno: async p => { const input=p.getByTestId("fixture-root").locator("input").first(); await input.fill("https://suno.com/@fixture"); await p.evaluate(()=> { window.__fixtureFaults={suno_parse_profile:{delay:250,error:"fixture parse failure"}}; }); await button(p,"解析").click(); await has(p,"fixture parse failure"); assert.equal((await p.evaluate(()=>window.__fixtureCalls)).filter(c=>c.command==="suno_parse_profile").length,1); },
 dbdesigner: async p => { await button(p,"新建").click(); await p.waitForFunction(()=>window.__fixtureCalls.some(c=>c.command==="dbd_validate")); assert.ok(await p.getByTestId("fixture-root").locator("input").evaluateAll(inputs=>inputs.some(input=>input.value==="隔离设计"))); },
};
(async()=>{
 const browser=await chromium.launch({channel:"msedge",headless:true});
 const newContext=async()=>{
  const context=await browser.newContext({viewport:{width:1280,height:800},locale:"zh-CN",timezoneId:"Asia/Shanghai"});
  context.setDefaultTimeout(30000); context.setDefaultNavigationTimeout(60000);
  await context.route("**/*",route=>{const url=new URL(route.request().url());return url.origin==="http://127.0.0.1:5196" || ["blob:","data:"].includes(url.protocol) ? route.continue():route.abort();});
  return context;
 };
 const selected=process.env.ACCEPT_MODULES ? order.filter(id=>process.env.ACCEPT_MODULES.split(",").includes(id)):order;
 const previous=process.env.ACCEPT_MODULES ? JSON.parse(await fs.readFile(path.join(output,"interactions.json"),"utf8").catch(()=>"[]")) : [];
 const results=previous.filter(result=>!selected.includes(result.id));
 try {
  for(let offset=0;offset<selected.length;offset+=3) {
   await Promise.all(selected.slice(offset,offset+3).map(async id=>{
    // Clock installation affects its context; isolate it from Monaco performance marks.
    const context=await newContext(),page=await context.newPage(),errors=[];
    page.on("pageerror",error=>errors.push(error.stack || String(error)));
    try {
     await page.goto(`http://127.0.0.1:5196/reports/module-ui-2026-11-02/preview/index.html?module=${id}`,{waitUntil:"domcontentloaded"});
     await page.waitForFunction(()=>{const root=document.querySelector('[data-testid="fixture-root"]');return root && root.innerText.trim() && root.innerText.trim()!=="加载中…";},null,{timeout:60000});
     await cases[id](page);
     const missing=await page.evaluate(()=>window.__fixtureMissing);
     assert.deepEqual(missing,[]); assert.deepEqual(errors,[]);
     await page.screenshot({path:path.join(output,`${id}-interaction.png`),animations:"disabled"});
     results.push({id,passed:true,calls:await page.evaluate(()=>window.__fixtureCalls)});
     console.log(id,"PASS");
    } catch(error) {
     results.push({id,passed:false,error:String(error),errors,missing:await page.evaluate(()=>window.__fixtureMissing).catch(()=>[]),buttons:await page.getByTestId("fixture-root").getByRole("button").allTextContents().catch(()=>[]),text:(await page.locator("body").innerText().catch(()=>"")).slice(0,4000)});
     console.log(id,"FAIL",String(error).slice(0,160));
    } finally {await context.close();}
   }));
   results.sort((a,b)=>order.indexOf(a.id)-order.indexOf(b.id));
   await fs.writeFile(path.join(output,"interactions.json"),JSON.stringify(results,null,2));
  }
 } finally {await browser.close();}
 if(results.some(result=>!result.passed)) process.exitCode=1;
})().catch(error=>{console.error(error);process.exitCode=1;});
