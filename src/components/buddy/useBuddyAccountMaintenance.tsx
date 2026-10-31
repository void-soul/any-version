import type { BuddyPlatformScope } from "./platformScope";
import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { selectRotationCandidate, type RotationAccount } from "./quota";
import { accountCreditSegments, parseQuotaItems, summarizeQuota, type BuddyAccount, type BuddyAccountView } from "./BuddyPanel.model";
import type { Dispatch, SetStateAction } from "react";

/** Owns the BuddyAccountMaintenance feature, independently of page rendering. */
export function useBuddyAccountMaintenance({ platformScope, showMsg, accounts, currentId, setBusy, setMessage, load, platform }: {
  platformScope: BuddyPlatformScope;
  showMsg: (ok: boolean, text: string) => void;
  accounts: BuddyAccount[];
  currentId: string | null;
  setBusy: Dispatch<SetStateAction<boolean>>;
  setMessage: Dispatch<SetStateAction<{ ok: boolean; text: string; } | null>>;
  load: () => Promise<void>;
  platform: string;
}) {
  const { t } = useTranslation();

  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  // 待删除账号：单个删除（卡片按钮）或批量删除（工具栏）共用同一个确认弹窗
  const [deleteIds, setDeleteIds] = useState<string[] | null>(null);

  const [usageAccountId, setUsageAccountId] = useState<string | null>(null);

  // 账号视图偏好：主账号 + 排序模式（后端 `<platform>_view.json` 持久化）
  const [accountView, setAccountView] = useState<BuddyAccountView>({ orderMode: "lastUsed" });

  // 主账号：点 ★ 设为/取消主账号（主账号恒排在列表首位）
  const togglePrimary = async (accountId: string) => {
    const lease = platformScope.capture();
    if (!lease.current()) return;
    try {
      const next = accountView.primaryAccountId === accountId ? null : accountId;
      const view = await invoke<BuddyAccountView>("buddy_set_primary_account", {
        platform,
        accountId: next,
      });
      if (!lease.current()) return;
      setAccountView(view);
    } catch (e) {
      if (!lease.current()) return;
      showMsg(false, String(e));
    }
  };

  const changeOrderMode = async (orderMode: string) => {
    const lease = platformScope.capture();
    if (!lease.current()) return;
    try {
      const view = await invoke<BuddyAccountView>("buddy_set_account_order_mode", {
        platform,
        orderMode,
      });
      if (!lease.current()) return;
      setAccountView(view);
    } catch (e) {
      if (!lease.current()) return;
      showMsg(false, String(e));
    }
  };

  const usageAccount = useMemo(
    () => accounts.find((a) => a.id === usageAccountId) ?? null,
    [accounts, usageAccountId]
  );

  // 全平台额度聚合（左上角总额度）
  const globalQuota = useMemo(() => {
    let remain = 0;
    let total = 0;
    let unlimited = false;
    let hasData = false;
    for (const a of accounts) {
      const q = summarizeQuota(parseQuotaItems(a.quotaRaw));
      if (!q.hasData) continue;
      hasData = true;
      if (q.unlimited) unlimited = true;
      remain += q.remain;
      total += q.total;
    }
    return { remain, total, unlimited, hasData };
  }, [accounts]);

  // 账号列表排序：主账号恒在首位，其余按排序模式（最近使用 / 剩余额度 / 最早到期）
  const sortedAccounts = useMemo(() => {
    const list = [...accounts];
    const quotaRemain = (a: BuddyAccount) => {
      const q = summarizeQuota(parseQuotaItems(a.quotaRaw));
      if (q.unlimited) return Number.POSITIVE_INFINITY;
      return q.hasData ? q.remain : -1;
    };
    const expiryAt = (a: BuddyAccount) => {
      const times = Object.values(a.expiryTimes ?? {})
        .filter((v) => Number.isFinite(v) && v > 0)
        .map((v) => (v < 1e12 ? v * 1000 : v));
      return times.length ? Math.min(...times) : Number.POSITIVE_INFINITY;
    };
    if (accountView.orderMode === "quota") {
      list.sort((a, b) => quotaRemain(b) - quotaRemain(a) || b.lastUsed - a.lastUsed);
    } else if (accountView.orderMode === "expiry") {
      list.sort((a, b) => expiryAt(a) - expiryAt(b) || b.lastUsed - a.lastUsed);
    } else {
      list.sort((a, b) => b.lastUsed - a.lastUsed);
    }
    const primary = accountView.primaryAccountId;
    if (primary) {
      const index = list.findIndex((a) => a.id === primary);
      if (index > 0) list.unshift(...list.splice(index, 1));
    }
    return list;
  }, [accounts, accountView]);

  // 当前账号积分不足 → 建议切换到哪个账号（参考 WorkDaddy credit-rotation 语义：
  // 排除当前账号与无剩余账号，最早到期优先，同到期取剩余多的）
  const rotationCandidate = useMemo(() => {
    if (!currentId) return null;
    const current = accounts.find((a) => a.id === currentId);
    if (!current) return null;
    const currentQuota = summarizeQuota(parseQuotaItems(current.quotaRaw));
    // 不限量 / 还有剩余 / 还没查到额度 → 不提示
    if (currentQuota.unlimited || !currentQuota.hasData || currentQuota.remain > 0) return null;
    // 传**含当前账号**的完整列表：`selectRotationCandidate` 自己排除当前账号，
    // 同时要用"当前账号自己的最近到期段"判断换过去到底有没有价值（参考 §7 规则）。
    const list: RotationAccount[] = accounts.map((a) => ({
      accountId: a.id,
      uid: a.uid ?? null,
      label: a.email || a.id,
      segments: accountCreditSegments(a),
    }));
    return selectRotationCandidate(list, current.uid ?? null, Date.now());
  }, [accounts, currentId]);

  const refreshAccount = async (id: string) => {
    const lease = platformScope.capture();
    if (!lease.current()) return;
    setBusy(true);
    setMessage(null);
    try {
      await invoke<BuddyAccount>("buddy_refresh_token", { platform, accountId: id });
      if (!lease.current()) return;
      showMsg(true, t("buddy.refreshed"));
      await load();
    } catch (e) {
      if (!lease.current()) return;
      showMsg(false, String(e));
    } finally {
      if (lease.current()) {
        setBusy(false);
      }
    }
  };

  const refreshSelected = async () => {
    const lease = platformScope.capture();
    if (!lease.current()) return;
    if (selectedIds.size === 0) return;
    setBusy(true);
    setMessage(null);
    try {
      const count = await invoke<number>("buddy_refresh_tokens", {
        platform,
        accountIds: [...selectedIds],
      });
      if (!lease.current()) return;
      showMsg(true, t("buddy.refreshedAll", { count }));
      await load();
    } catch (e) {
      if (!lease.current()) return;
      showMsg(false, String(e));
    } finally {
      if (lease.current()) {
        setBusy(false);
      }
    }
  };

  const deleteAccounts = async (ids: string[]) => {
    const lease = platformScope.capture();
    if (!lease.current()) return;
    setBusy(true);
    setMessage(null);
    try {
      await invoke("buddy_delete_accounts", { platform, accountIds: ids });
      if (!lease.current()) return;
      setSelectedIds((prev) => {
        const next = new Set(prev);
        ids.forEach((id) => next.delete(id));
        return next;
      });
      setDeleteIds(null);
      await load();
    } catch (e) {
      if (!lease.current()) return;
      showMsg(false, String(e));
    } finally {
      if (lease.current()) {
        setBusy(false);
      }
    }
  };

  // 导出指定账号（工具栏批量导出 / 卡片单个导出共用）
  const exportByIds = async (ids: string[], nameBase?: string) => {
    const lease = platformScope.capture();
    if (!lease.current()) return;
    if (ids.length === 0) return;
    try {
      const json = await invoke<string>("buddy_export_accounts", { platform, accountIds: ids });
      if (!lease.current()) return;
      const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
      const name = `${nameBase ?? platform}-accounts-${ts}.json`;
      await invoke("file_io_save_text", { fileName: name, content: json });
      if (!lease.current()) return;
      showMsg(true, t("buddy.exported", { name }));
    } catch (e) {
      if (!lease.current()) return;
      showMsg(false, String(e));
    }
  };

  const exportSelected = async () => exportByIds([...selectedIds]);

  const toggleSelect = (id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // 全选 / 取消全选：已全选则清空，否则勾上当前全部账号
  const toggleSelectAll = () => {
    setSelectedIds((prev) =>
      accounts.length > 0 && prev.size === accounts.length
        ? new Set()
        : new Set(accounts.map((a) => a.id)),
    );
  };

  const selectedCount = selectedIds.size;
  return {
    selectedIds,
    setSelectedIds,
    deleteIds,
    setDeleteIds,
    setUsageAccountId,
    accountView,
    setAccountView,
    togglePrimary,
    changeOrderMode,
    usageAccount,
    globalQuota,
    sortedAccounts,
    rotationCandidate,
    refreshAccount,
    refreshSelected,
    deleteAccounts,
    exportSelected,
    toggleSelect,
    toggleSelectAll,
    selectedCount,
  };
}
