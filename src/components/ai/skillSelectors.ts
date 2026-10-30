import type { SkillEntry } from "./SkillManager.model";

/** Category and tag facets share the same library snapshot. */
export function getSkillFacets(skills: SkillEntry[]) {
  const categories = new Set<string>();
  const tags = new Set<string>();
  for (const skill of skills) {
    if (skill.category) categories.add(skill.category);
    for (const tag of skill.tags) tags.add(tag);
  }
  return { allCats: [...categories].sort(), allTags: [...tags].sort() };
}
