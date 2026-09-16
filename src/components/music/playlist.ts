// 播放列表推进逻辑（纯函数，便于单测）。
//
// 三种模式：
// - sequence：循环下一首；
// - single：重复当前曲目；
// - shuffle：洗牌袋随机（袋空后重新洗牌，并保证不与当前曲重复，避免"随机到同一首"的观感）。
//
// history 记录播放过的曲目（含随机模式下实际播放的顺序），供「上一首」回退。

export type PlayMode = "sequence" | "shuffle" | "single";

export interface PlaylistCursor {
  /** 洗牌袋：已打乱、尚未播放的索引 */
  bag: number[];
  /** 播放历史（不含当前曲目） */
  history: number[];
}

export interface AdvanceResult {
  /** 下一个索引；列表为空时为 null */
  index: number | null;
  cursor: PlaylistCursor;
}

export function createCursor(): PlaylistCursor {
  return { bag: [], history: [] };
}

/** Fisher-Yates 洗牌；`exclude` 会被挪出首位，防止刚洗完就重复当前曲 */
export function shuffleIndices(
  count: number,
  random: () => number = Math.random,
  exclude?: number | null
): number[] {
  const indices = Array.from({ length: count }, (_, i) => i);
  for (let i = count - 1; i > 0; i -= 1) {
    const j = Math.floor(random() * (i + 1));
    [indices[i], indices[j]] = [indices[j], indices[i]];
  }
  if (exclude != null && count > 1 && indices[0] === exclude) {
    [indices[0], indices[1]] = [indices[1], indices[0]];
  }
  return indices;
}

interface AdvanceOptions {
  current: number | null;
  total: number;
  mode: PlayMode;
  cursor: PlaylistCursor;
  random?: () => number;
}

/** 下一首（当前曲目会被压入历史） */
export function nextTrack({ current, total, mode, cursor, random }: AdvanceOptions): AdvanceResult {
  if (total <= 0) {
    return { index: null, cursor };
  }

  let next: number;
  let bag = cursor.bag;
  const history = current == null ? [...cursor.history] : [...cursor.history, current];

  if (mode === "single" && current != null) {
    next = current;
  } else if (mode === "shuffle") {
    bag = cursor.bag.length > 0 ? cursor.bag : shuffleIndices(total, random, current);
    next = bag[0];
    bag = bag.slice(1);
  } else {
    next = current == null ? 0 : (current + 1) % total;
  }

  return { index: next, cursor: { bag, history } };
}

/** 上一首（从历史回退，不恢复洗牌袋） */
export function prevTrack({ current, total, mode, cursor }: AdvanceOptions): AdvanceResult {
  if (total <= 0) {
    return { index: null, cursor };
  }

  if (mode === "single" && current != null) {
    return { index: current, cursor };
  }

  const history = [...cursor.history];
  const previous = history.pop();
  if (previous != null && previous < total) {
    return { index: previous, cursor: { bag: cursor.bag, history } };
  }

  // 没有历史（例如刚打开）：顺序模式退一格，随机模式从头开始
  const fallback = mode === "sequence" && current != null ? (current - 1 + total) % total : 0;
  return { index: fallback, cursor: { bag: cursor.bag, history } };
}
