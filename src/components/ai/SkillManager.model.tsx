
// ─── 类型 ───
/** 预置技能来源（后端 get_skill_sources） */
export interface SkillSourceItem {
  label: string;
  repo: string;
  hint: string;
}

/** 有更新的技能（后端 check_skill_updates；camelCase） */
export interface SkillUpdateInfo {
  skillId: string;
  name: string;
  source: string;
  current: string;
  latest: string;
}

export interface SkillEntry {
  id: string;
  name: string;
  description: string;
  installedAt: string;
  installMethod: string;
  /** 用户自定义分类（来自 .meta.json） */
  category: string;
  /** 用户自定义标签（来自 .meta.json） */
  tags: string[];
  /** SKILL.md 字符数：描述每次会话常驻，这个数字就是该技能的成本量级 */
  skillMdChars: number;
}

/** 内置技能（随 Kira 分发，装进技能库后由用户安装给项目） */
export interface BuiltinSkillView {
  id: string;
  name: string;
  description: string;
}

/** 项目 = 一个目录，技能装给它就只在该目录生效（后端 list_projects / add_project） */
export interface SkillProject {
  id: string;
  path: string;
  name: string;
  addedAt: string;
}

/** 安装结果（后端 InstallOutcome）：逐目录成败，不是「一句装好了」 */
export interface InstallOutcome {
  linked: string[];
  failed: string[];
  replaced: string[];
}

/** 项目里已安装的技能（后端 list_project_skills；camelCase） */
export interface ProjectSkillEntry {
  skillId: string;
  name: string;
  description: string;
  /**
   * 'linked' 指向技能库的链接 · 'local' 项目自带的真实目录 · 'broken' 悬空链接
   * （技能库里的技能已被删除，链接还留着）
   */
  state: string;
  skillMdChars: number;
  /** 技能库 .meta.json 里的标签/分类 —— 在项目列表里同样显示，免得两边信息各说各话 */
  tags: string[];
  category: string;
  /** 装在哪些扫描目录（一个技能装 N 处也只有**一行**，dirs 说明装在哪） */
  dirs?: string[];
  /** 这些链接里有多少已失效（目标技能被从库里删了）；有一个还有效就仍算「已安装」 */
  staleLinks?: number;
}

export type TabKey = 'skills' | 'projects';

export const MARKET_SOURCES = [
  { name: 'skills.sh 官网', desc: 'Agent Skills 官方生态与规范', url: 'https://skills.sh' },
  { name: 'Anthropic 官方 Skills', desc: 'anthropics/skills 仓库示例技能', url: 'https://github.com/anthropics/skills' },
  { name: 'GitHub 主题搜索', desc: '搜索社区维护的 agentskills 仓库', url: 'https://github.com/search?q=agentskills&type=repositories' },
];
