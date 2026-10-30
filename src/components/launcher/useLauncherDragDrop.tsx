import { useState, useEffect, useRef } from "react";
import { PointerSensor, KeyboardSensor, useSensor, useSensors } from "@dnd-kit/core";
import type { DragStartEvent, DragOverEvent, DragEndEvent } from "@dnd-kit/core";
import { sortableKeyboardCoordinates } from "@dnd-kit/sortable";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useTranslation } from "react-i18next";
import { Classification, Item } from "./types";
import { buildReorderOrders } from "./reorder";
import type { Dispatch, SetStateAction, RefObject } from "react";

/** Owns the LauncherDragDrop feature, independently of page rendering. */
export function useLauncherDragDrop({ topCategories, activeTopCategory, setAllItems, showToast, setActiveParentId, allItems, allItemsRef, classificationMap, loadData }: {
  topCategories: Classification[];
  activeTopCategory: Classification;
  setAllItems: Dispatch<SetStateAction<Item[]>>;
  showToast: (msg: string) => void;
  setActiveParentId: Dispatch<SetStateAction<number | null>>;
  allItems: Item[];
  allItemsRef: RefObject<Item[]>;
  classificationMap: Map<number, Classification>;
  loadData: () => Promise<void>;
}) {
  const { t } = useTranslation();


  // Drag & Drop State
  const [isDragOver, setIsDragOver] = useState(false);

  const [targetDragSubId, setTargetDragSubId] = useState<number | null>(null);


  // 项目卡片内部拖拽（dnd-kit：拖放修改分类 / 排序）
  const [activeDragItem, setActiveDragItem] = useState<Item | null>(null);

  const draggingItemRef = useRef<Item | null>(null);

  // 拖拽开始时被拖项目的原始分类（跨分类移动持久化时需要）
  const dragSourceCatIdRef = useRef<number | null>(null);

  // 本轮拖拽是否真的改变了界面上的顺序（由 applyMoveLocal 置位）。
  // 持久化以「界面看到的顺序」为准，而不是重新解析 dragEnd 的 over：
  // 指针松手时可能落在卡片间隙/容器上（over=cat:*），此时若按 over 推导会漏保存，
  // 表现为「拖拽时顺序让位了，松手后重进又变回原样」。
  const movedDuringDragRef = useRef(false);


  // dnd-kit 传感器：移动超过 6px 才判定为拖拽，单击仍可正常启动应用
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );


  // WebView2 中轻拖（按下即释放）有时会补发 click，导致拖拽结束误启动应用。
  // 记录最近一次拖拽结束时间，短暂忽略随后的 click。
  const justDraggedAtRef = useRef(0);


  // 顶层分类 id 集合（Tauri 原生文件拖入左栏时判断是否需要切换分类）
  const topCategoryIdsRef = useRef<Set<number>>(new Set());

  useEffect(() => {
    topCategoryIdsRef.current = new Set(topCategories.map((c) => c.id));
  }, [topCategories]);


  // Process dropped paths with deduplication
  const isDroppingRef = useRef(false);

  const lastDropKeyRef = useRef("");

  const lastDropTimeRef = useRef(0);


  const activeTopCategoryRef = useRef<Classification | undefined>(undefined);


  useEffect(() => {
    activeTopCategoryRef.current = activeTopCategory;
  }, [activeTopCategory]);


  const processDroppedPaths = async (paths: string[], targetClassificationId: number) => {
    if (!paths || paths.length === 0) return;
    const now = Date.now();
    const dropKey = `${paths.slice().sort().join("|")}-${targetClassificationId}`;
    if (dropKey === lastDropKeyRef.current && now - lastDropTimeRef.current < 1200) {
      return;
    }
    lastDropKeyRef.current = dropKey;
    lastDropTimeRef.current = now;

    if (isDroppingRef.current) return;
    isDroppingRef.current = true;

    try {
      const newItems = await invoke<Item[]>("launcher_process_dropped_paths", {
        paths,
        classificationId: targetClassificationId,
      });
      // 局部更新：只追加本次新增的项目，避免整页数据重载
      // （重载会重置当前分类/滚动/展开状态，且历史上会因闭包过期把激活分类跳回第一个）
      if (newItems && newItems.length > 0) {
        setAllItems((prev) => [...prev, ...newItems]);
      }
      showToast(t("launcher.addedItems", { count: newItems.length }));
    } catch (err: any) {
      console.error("处理拖入路径失败:", err);
      showToast(t("launcher.addFail", { err: String(err) }));
    } finally {
      isDroppingRef.current = false;
      setIsDragOver(false);
      setTargetDragSubId(null);
    }
  };


  // 常驻监听器（onDragDropEvent 只在挂载时注册一次）通过该 ref 调用最新版本的 processDroppedPaths，
  // 避免闭包捕获到首次渲染的旧函数（旧闭包里的 loadData 会因 activeParentId 为 null 而重置当前分类）
  const processDroppedPathsRef = useRef<typeof processDroppedPaths>(processDroppedPaths);

  useEffect(() => {
    processDroppedPathsRef.current = processDroppedPaths;
  });


  // 根据光标位置（CSS 像素）解析当前悬停的子分组
  // Tauri 原生文件拖拽不会触发 DOM 的 HTML5 dragover/drop 事件，
  // 只能从 onDragDropEvent 的 position（物理像素）反查 DOM，做到「放到哪个二级分类就加到哪个」
  const detectDragSubIdFromPoint = (clientX: number, clientY: number): number | null => {
    try {
      const el = document.elementFromPoint(clientX, clientY);
      if (!el) return null;
      const groupEl = el.closest("[data-cat-id]");
      if (!groupEl) return null;
      const raw = groupEl.getAttribute("data-cat-id");
      const id = raw ? Number(raw) : NaN;
      return Number.isFinite(id) ? id : null;
    } catch {
      return null;
    }
  };


  // Native Tauri Window Drag & Drop listener (registered once on mount)
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setupTauriDragDrop = async () => {
      try {
        // 注意：Tauri 的文件拖放事件（tauri://drag-*）在「Webview」级别发射，
        // 必须用 getCurrentWebview() 监听（target kind=Webview）；
        // 用 getCurrentWindow()（kind=Window）会因 target 不匹配而收不到事件，
        // 表现为拖入文件时光标始终是「禁止」且 onDragDropEvent 从不触发。
        const win = getCurrentWebview();
        unlisten = await win.onDragDropEvent(async (event) => {
          if (event.payload.type === "enter" || event.payload.type === "over") {
            setIsDragOver(true);
            // 实时按光标位置更新悬停的子分组（同时驱动分组高亮反馈）
            const pos = event.payload.position;
            if (pos && typeof pos.x === "number" && typeof pos.y === "number") {
              const scale = window.devicePixelRatio || 1;
              setTargetDragSubId(detectDragSubIdFromPoint(pos.x / scale, pos.y / scale));
            }
          } else if (event.payload.type === "leave") {
            setIsDragOver(false);
            setTargetDragSubId(null);
          } else if (event.payload.type === "drop") {
            setIsDragOver(false);
            const paths = event.payload.paths;
            if (paths && paths.length > 0) {
              // drop 时刻以光标位置为准，避免悬停状态滞后导致放错分类
              let subId: number | null = null;
              const pos = event.payload.position;
              if (pos && typeof pos.x === "number" && typeof pos.y === "number") {
                const scale = window.devicePixelRatio || 1;
                subId = detectDragSubIdFromPoint(pos.x / scale, pos.y / scale);
              }
              const targetId =
                subId !== null
                  ? subId
                  : activeTopCategoryRef.current
                    ? activeTopCategoryRef.current.id
                    : 1;
              await processDroppedPathsRef.current(paths, targetId);
              // 拖到左栏的大分类上：切过去让用户立即看到添加结果
              if (subId !== null && topCategoryIdsRef.current.has(subId)) {
                setActiveParentId(subId);
              }
            }
            setTargetDragSubId(null);
          }
        });
      } catch (e) {
        console.error("Tauri drag-drop listener registration error:", e);
      }
    };

    setupTauriDragDrop();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);


  // HTML5 Drag & Drop Fallback Handlers（仅处理外部文件拖入；内部项目拖拽走 dnd-kit）
  const handleHtml5DragOver = (e: React.DragEvent, subId?: number) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(true);
    if (subId) setTargetDragSubId(subId);
  };


  const handleHtml5DragLeave = (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);
  };


  const handleHtml5Drop = async (e: React.DragEvent, subId?: number) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);
    const targetId = subId || targetDragSubId || (activeTopCategory ? activeTopCategory.id : 1);
    const files = e.dataTransfer.files;
    if (files && files.length > 0) {
      const paths: string[] = [];
      for (let i = 0; i < files.length; i++) {
        // @ts-ignore
        const fullPath = files[i].path || files[i].name;
        if (fullPath) paths.push(fullPath);
      }
      if (paths.length > 0) {
        await processDroppedPaths(paths, targetId);
      }
    }
  };


  // ---- dnd-kit：内部项目拖拽（移动分类 / 排序）----

  const findItemById = (id: string): Item | undefined => {
    if (!id.startsWith("item:")) return undefined;
    const num = Number(id.slice(5));
    return allItems.find((it) => it.id === num);
  };


  // 纯本地状态更新：把 dragItem 移动到 targetCatId 的 insertBeforeId 之前（null = 末尾）
  // 用于 onDragOver 的实时让位；不写后端（避免拖拽过程中狂发请求）
  const applyMoveLocal = (dragItem: Item, targetCatId: number, insertBeforeId: number | null) => {
    setAllItems((prev) => {
      const current = prev.find((it) => it.id === dragItem.id) || dragItem;
      const sourceId = current.classificationId;
      if (sourceId === targetCatId && insertBeforeId === null) return prev;

      const movedItem = { ...current, classificationId: targetCatId };
      const byCat = new Map<number, Item[]>();
      for (const it of prev) {
        if (it.id === movedItem.id) continue;
        const arr = byCat.get(it.classificationId) || [];
        arr.push(it);
        byCat.set(it.classificationId, arr);
      }

      const target = byCat.get(targetCatId) || [];
      let newTarget: Item[];
      if (insertBeforeId === null) {
        newTarget = [...target, movedItem];
      } else {
        const idx = target.findIndex((it) => it.id === insertBeforeId);
        newTarget =
          idx === -1 ? [...target, movedItem] : [...target.slice(0, idx), movedItem, ...target.slice(idx)];
      }
      byCat.set(targetCatId, newTarget);

      const flat: Item[] = [];
      for (const arr of byCat.values()) flat.push(...arr);

      // 顺序没变（例如拖回原位）时不算移动，避免多写一次数据库
      const sameOrder =
        flat.length === prev.length && flat.every((it, i) => it.id === prev[i].id);
      if (sameOrder) return prev;

      // 立即同步 ref：dragEnd 持久化时读到的必须是「界面最终看到的顺序」，
      // 不能依赖 useEffect（被动副作用可能晚于 dnd-kit 的 dragEnd 回调）。
      allItemsRef.current = flat;
      movedDuringDragRef.current = true;
      return flat;
    });
  };


  // 拖拽结束后把最终位置持久化到后端（保存分类 + 重排序号）。
  // 持久化的唯一依据是 allItemsRef.current —— 即拖拽过程中 applyMoveLocal 已经
  // 让位好的「界面所见顺序」，不再重新解析 dragEnd 的 over。
  // 原因：松手瞬间指针可能落在卡片间隙/容器上（over=cat:*）或容器之外（over=null），
  // 按 over 推导会漏保存或把项目挪到末尾，表现为「拖完顺序让位了，重开又变回原样」。
  // sourceCatId 由 handleDragEnd 在清空 dragSourceCatIdRef 之前传入（跨分类时需要）。
  const persistMoveAfterDrag = async (dragItem: Item, sourceCatId: number | null) => {
    try {
      // 必须用完整列表（含被「只显示有效项目」隐藏的项）重排序号：
      // 只用可见项会让隐藏项保留旧序号并与新序号冲突，重新打开后顺序错乱。
      const fullList = allItemsRef.current;
      const moved = fullList.find((it) => it.id === dragItem.id);
      if (!moved) return;
      const targetCatId = moved.classificationId;
      const sourceId = sourceCatId ?? targetCatId;

      if (sourceId !== targetCatId) {
        // 跨分类：先把分类写回数据库（序号随后由下面的整体重排覆盖）
        await invoke("launcher_save_item", {
          item: { ...moved, classificationId: targetCatId },
        });
      }

      // 按本地最终顺序重排受影响分类的序号
      const affectedCats = sourceId === targetCatId ? [targetCatId] : [targetCatId, sourceId];
      const orderMap = new Map<number, number>();
      for (const { orders } of buildReorderOrders(fullList, affectedCats)) {
        if (orders.length === 0) continue;
        for (const [id, order] of orders) orderMap.set(id, order);
        await invoke("launcher_reorder_items", { orders });
      }

      // 同步本地 order，避免后续保存（编辑 / 检测）把旧序号写回数据库导致顺序回退
      if (orderMap.size > 0) {
        setAllItems((prev) =>
          prev.map((it) => (orderMap.has(it.id) ? { ...it, order: orderMap.get(it.id)! } : it))
        );
      }
    } catch (err) {
      showToast(t("launcher.saveOrderFail", { err: String(err) }));
    }
  };


  const handleDragStart = (event: DragStartEvent) => {
    const item = findItemById(String(event.active.id));
    if (!item) return;
    setActiveDragItem(item);
    draggingItemRef.current = item;
    dragSourceCatIdRef.current = item.classificationId;
    movedDuringDragRef.current = false;
  };


  const handleDragOver = (event: DragOverEvent) => {
    const { active, over } = event;
    if (!over) {
      setTargetDragSubId(null);
      return;
    }
    const activeId = String(active.id);
    const overId = String(over.id);
    if (activeId === overId) return;

    const activeItem = findItemById(activeId);
    if (!activeItem) return;

    let targetCatId: number | null = null;
    let insertBeforeId: number | null = null;

    if (overId.startsWith("item:")) {
      const overItem = findItemById(overId);
      if (!overItem || overItem.id === activeItem.id) return;
      targetCatId = overItem.classificationId;
      insertBeforeId = overItem.id;
    } else if (overId.startsWith("cat:") || overId.startsWith("sidebar:")) {
      targetCatId = Number(overId.slice(overId.indexOf(":") + 1));
    }
    if (targetCatId === null || !Number.isFinite(targetCatId)) return;
    if (!classificationMap.has(targetCatId)) return;

    setTargetDragSubId(targetCatId); // 分组 / 左栏高亮

    if (activeItem.classificationId === targetCatId && insertBeforeId === null) return;
    applyMoveLocal(activeItem, targetCatId, insertBeforeId);
  };


  const handleDragEnd = async (event: DragEndEvent) => {
    const { active, over } = event;
    // 先取到本次拖拽的上下文，再清空 ref
    const sourceCatId = dragSourceCatIdRef.current;
    const moved = movedDuringDragRef.current;
    setActiveDragItem(null);
    draggingItemRef.current = null;
    dragSourceCatIdRef.current = null;
    movedDuringDragRef.current = false;
    setTargetDragSubId(null);
    justDraggedAtRef.current = Date.now(); // 拦截拖拽结束后的残留 click

    // 拖到左栏大分类：切过去让用户立即看到结果（即使本次没有改变顺序）
    const overId = over ? String(over.id) : "";
    if (overId.startsWith("sidebar:")) {
      const catId = Number(overId.slice(overId.indexOf(":") + 1));
      if (Number.isFinite(catId) && classificationMap.has(catId)) setActiveParentId(catId);
    }

    // applyMoveLocal 从未真正改变顺序（拖回原位 / 拖到空白处）→ 无需写库。
    // 这里不看 dragEnd 的 over：松手时指针可能落在卡片间隙(over=cat:*)甚至容器外(over=null)，
    // 但界面上已经让位出的顺序就是用户想要的结果，必须按它保存。
    if (!moved) return;

    const activeIdNum = Number(String(active.id).slice("item:".length));
    const activeItem = allItemsRef.current.find((it) => it.id === activeIdNum);
    if (!activeItem) return;

    await persistMoveAfterDrag(activeItem, sourceCatId);
  };


  const handleDragCancel = async () => {
    const moved = movedDuringDragRef.current;
    setActiveDragItem(null);
    draggingItemRef.current = null;
    dragSourceCatIdRef.current = null;
    movedDuringDragRef.current = false;
    setTargetDragSubId(null);
    justDraggedAtRef.current = Date.now();
    // 取消（ESC）时本地已让位但从未写库，重载回滚，避免界面与数据库不一致
    if (moved) await loadData();
  };
  return {
    isDragOver,
    setIsDragOver,
    targetDragSubId,
    setTargetDragSubId,
    activeDragItem,
    sensors,
    justDraggedAtRef,
    processDroppedPaths,
    handleHtml5DragOver,
    handleHtml5DragLeave,
    handleHtml5Drop,
    handleDragStart,
    handleDragOver,
    handleDragEnd,
    handleDragCancel,
  };
}
