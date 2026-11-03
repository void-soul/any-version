// Isolated fixture: no Tauri bridge, network, user data or OS mutation.
import type { AiConfig } from "../../../src/contracts/ai";
import type { MusicLibrary, MusicSettings, PlayerState, PluginListResult } from "../../../src/components/music/types";
import type { FavoriteSettings, FavoriteStats } from "../../../src/components/favorites/types";
export const calls: Array<{command:string;args:unknown}> = [];
(window as any).__fixtureCalls = calls;
const missing: string[] = [];
(window as any).__fixtureMissing = missing;
const appearance = { moduleThemeColors:{}, globalFont:"", customFontPath:"", moduleOrder:[], toolbarModules:[], disabledModules:[], backgroundTexture:"none",language:"zh",motionPreference:"system",density:"cozy" };
const eq = { enabled:false,bands:Array(10).fill(0),gain_db:0,balance:0,preset:"flat" };
const settings: MusicSettings = {volume:0.8,play_mode:"sequence",eq,download_dir:""};
const player: PlayerState = {status:"idle",path:null,title:null,artist:null,position_ms:0,duration_ms:0,volume:0.8,online:null};
const library: MusicLibrary = {folders:["C:/fixture/music"],tracks:[{path:"C:/fixture/music/sample.mp3",folder:"C:/fixture/music",title:"示例歌曲",artist:"示例歌手",album:"隔离验收",duration_ms:185000,size_bytes:123456}]};
const aiConfig: AiConfig = {providers:[],proxy_port:0,default_project_path:"",rectifier:{enabled:true,thinking_signature:true,thinking_budget:true,media_fallback:true,media_heuristic:true,protocol_mismatch:true,toolcall_dialect:true},headroom:{enabled:false,port:8787,on_unavailable:"failOpen",disable_kompress:false,timeout_ms:1000},optimizer:{enabled:false,cache_injection:false,thinking_optimizer:false,deepseek_normalize:false},skills_dir:""};
const proxyConfig = {control_dns:true,control_sniff:true,use_nameserver_policy:false,diff_work_dir:false,sys_proxy_enable:false,auto_start_core:false,auto_set_proxy:false,tun_enabled:false,tun_name:"fixture",mixed_port:7890,controller_port:9090,secret:"",core_path:null,default_proxy:null,secondary_proxies:[],secondary_active_id:null,proxy_cols:6,proxy_sort_type:"Default",keep_profile_alive:true,theme:"system",lang:"zh",ipc_port:null,sys_proxy_bypass:"",substore_enabled:false,webdav_url:"",webdav_user:"",webdav_pass:"",webdav_auto_backup:false,current_profile:"default",auto_close_proxy:false,auto_update_profile_on_start:false,ssid_switch_enabled:false,ssid_rules:[]};
const fixtures: Record<string,unknown> = {
 dbd_new_document:{id:"fixture-design",name:"隔离设计",description:"",dialect:"mysql",commonFields:[],decisions:[],nodes:[],relations:[]},
 dbd_get_session_index:{sessions:[]},dbd_get_chat:{chatMsgs:[],aiHistory:[],aiClarifyRound:0},dbd_save_chat:null,dbd_save_session_index:null,dbd_upsert_session_entry:null,dbd_validate:{errors:[],warnings:[]},
 mm_get_explorer_settings:{explorerRounds:5,explorerFilesPerRound:6,explorerCharsPerFile:12000,explorerBatchChars:60000,agentRounds:20,lastProviderId:null,lastModelId:null},
 markdown_assoc_status:{md:false,markdown:false,exePath:"C:/fixture/kira.exe"},take_pending_markdown_files:[],dbd_get_settings:{lastProviderId:null,lastModelId:null},
 get_rss_config:{sources:[],is_first_launch:false},list_env_backups:[],
 tasks_init:null,tasks_list_range:[],tasks_list_by_date:[],
 buddy_get_expiry_columns:[],api_init:null,mm_init:null,
 clipboard_get_items:{items:[],total:0},clipboard_get_settings:{enabled:true,maxItems:100,storeImages:true,ignoreBlank:true,ignoreShort:false},clipboard_get_ignored_apps:[],
 get_translate_config:{providerId:null,modelId:null,targetLang:"中文"},translate_history_list:[],
 picky_get_state:{bookmarks:[],comments:[],tags:[],bookmarkTags:{}},
 net_adapters:[],net_traffic:{ts_ms:0,adapters:[]},get_running_http_servers:[],get_path_directories:[],
 get_rtsp_camera_devices:{videoDevices:[],audioDevices:[]},get_all_local_ips:[],get_all_rtsp_server_statuses:[],
 mihomo_get_app_config:proxyConfig,mihomo_get_controled_config:{},mihomo_controller_info:{port:9090,secret:""},
 mihomo_get_state:{installed:false,core_version:null,running:false,pid:null,app_config:proxyConfig,current_profile:"default",runtime_config:"",controller_port:9090,secret:"",is_admin:false,warnings:[]},
 get_appearance_config:appearance,get_config:{data_dir:"C:/fixture/kira"},get_data_dir_cmd:"C:/fixture/kira",get_app_version:"2.9.5",
 get_tray_menu_config:{enabled:true,show_mihomo:true},launcher_get_settings:{moduleHotkeys:{},itemIconSize:32,webSearchSources:[]},
 buddy_get_current_account_id:null,buddy_get_paths:{platform:"codebuddy-cn",dataDir:null,stateDb:null,authFile:null},buddy_get_account_view:{primaryAccountId:null,orderMode:"lastUsed"},
 project_get_github_token:"",get_auto_start_services:[],is_admin:false,get_provider_presets:[],
 music_get_library:library,music_get_settings:settings,music_get_state:player,music_list_presets:[],
 music_plugin_list:{plugins:[],dir:"C:/fixture/plugins",deps:{ready:false,node_found:false,node_path:null,npm_path:null,missing_packages:[],root:"C:/fixture",problem:"隔离样例未安装插件依赖",sandbox:{enabled:false,reason:"隔离验收"}}} satisfies PluginListResult,music_track_name_suggestion:{current_name:"sample.mp3",suggested_name:"标签建议.mp3",from_tags:true},
 fav_stats:{total:0,unclassified:0,gone:0,bySource:[],tags:[],categories:[]} satisfies FavoriteStats,fav_get_settings:{leftWidth:220,providerId:null,modelId:null,agentRounds:20,aiWidth:380,aiOpen:false} satisfies FavoriteSettings,fav_get_github_token:"",fav_has_credential:false,fav_credential_status:[],
 get_ai_config:aiConfig,
};
// Explicit empty-list contracts only. Unknown commands always fail visibly.
for (const command of ["launcher_get_classifications","launcher_get_all_items","buddy_list_accounts","buddy_list_session_conflicts","project_list_fast","npm_list_projects","mm_list_documents","mm_list_folders","api_list_projects","api_list_environments","api_list_modules","api_list_endpoints","api_list_preset_headers","cert_list","credential_list","deploy_node_list","otp_list","otp_list_categories","serial_list_ports","suno_list_profiles","list_system_fonts","fav_list","fav_list_deleted"]) fixtures[command] = [];
(window as any).__fixtureResponses = fixtures;
Object.assign(fixtures, (window as any).__fixtureOverrides || {});
export async function invoke<T>(command:string,args?:unknown):Promise<T> {
 calls.push({command,args:structuredClone(args)}); window.dispatchEvent(new Event("fixture-call"));
 const fault = (window as any).__fixtureFaults?.[command];
 if (fault?.delay) await new Promise(resolve=>setTimeout(resolve,fault.delay));
 if (fault?.error) throw new Error(fault.error);
 if (command === "buddy_get_paths") return {platform:(args as any).platform,dataDir:null,stateDb:null,authFile:null} as T;
 if (command === "music_plugin_search") return {hits:[{file:"fixture.js",platform:"隔离音源",item:{id:"fixture",title:(args as any).keyword,artist:"搜索参数核验",duration:120}}],failures:[],exhausted:["fixture.js"]} as T;
 if (command === "music_track_name_suggestion") await new Promise(resolve=>setTimeout(resolve,1500));
 if (command in fixtures) return structuredClone(fixtures[command]) as T;
 if (command === "music_update_settings") return structuredClone(Object.assign(settings, (args as any).settings)) as T;
 if (command === "music_set_volume") return { ...player, volume: (args as any).volume } as T;
 if (command === "music_preview_eq") return structuredClone((args as any).eq) as T;
 if (command === "music_set_queue") return null as T;
 missing.push(command);
 throw new Error("隔离样例未配置："+command);
}
export const convertFileSrc = (_path:string) => "data:,";
export const isTauri = () => false;
const listeners = new Map<string, Set<(event:any)=>void>>();
export const listen = async (event:string, handler:(event:any)=>void) => {
 const group=listeners.get(event) || new Set(); group.add(handler); listeners.set(event,group);
 return () => { group.delete(handler); if(!group.size) listeners.delete(event); };
};
export const emit = async (event:string, payload?:unknown) => { for(const handler of listeners.get(event) || []) handler({event,id:0,payload}); };
(window as any).__fixtureEmit = emit;
(window as any).__fixtureListenerCount = (event:string) => listeners.get(event)?.size || 0;
export const open = async (..._args:unknown[]) => (window as any).__fixtureDialogs?.shift() ?? null;
export const save = async (..._args:unknown[]) => (window as any).__fixtureDialogs?.shift() ?? null;
export const openPath = async (..._args:unknown[]) => {};
export const openUrl = async (..._args:unknown[]) => {};
export const relaunch = async () => {};
export const check = async () => null;
export const enable = async () => {};
export const disable = async () => {};
export const isEnabled = async () => false;
export class LogicalSize { constructor(public width:number,public height:number) {} }
const win = {label:"preview",listen,onDragDropEvent:async (_handler:unknown)=>()=>{},hide:async()=>{},show:async()=>{},setSize:async()=>{},setFocus:async()=>{}};
export const getCurrentWindow = () => win;
export const getCurrentWebview = () => win;
