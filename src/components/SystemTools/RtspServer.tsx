import { RtspEncodingOptions } from "./RtspEncodingOptions";
import { useRtspServer } from "./useRtspServer";
import { btnPrimary, btnDanger, btnSecondary } from "../shared/Button";
import { ConfirmDialogHost } from "../shared/ConfirmDialog";
import { Video, Play, Square, RefreshCw, FolderOpen, Copy, CheckCircle, AlertCircle, Terminal, ChevronDown, ChevronUp, ChevronRight, Plus, Trash2, Radio, Sliders, Network, Mic, MicOff } from "lucide-react";

export default function RtspServer() {
  const { panelError, batchBusy, confirmRequest, setConfirmRequest, t, devices, loadingDevices, copiedUrl, allIps, loadAllIps, instances, setInstances, fetchDevices, handleAddInstance, handleDeleteInstance, handleStopAll, handleStartAll, toggleCollapse, updateInstanceConfig, handleCopyConfigToAll, handleStartInstance, handleStopInstance, handleSelectFile, handleCopy, formatUptime } = useRtspServer();
  const runningCount = instances.filter((i) => i.status.running).length;

  return (
    <div className="flex h-full min-h-0 w-full max-w-[1100px] flex-col overflow-hidden px-6 py-4 mx-auto space-y-5 select-none text-slate-200">
      {/* 头部控制栏固定，实例列表单独滚动 */}
      <div className="flex shrink-0 flex-col md:flex-row md:items-center justify-between gap-4 border-b border-white/10 pb-4">
        <div className="flex items-center gap-3">
          <div className="p-2.5 rounded-card bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] shadow-lg shadow-[var(--module-accent-ring)]">
            <Video className="w-6 h-6" />
          </div>
          <div>
            <h3 className="text-title font-bold text-white flex items-center gap-2">
              {t("rtsp.title")}
              {runningCount > 0 && (
                <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-caption font-medium bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-[var(--module-accent)] border border-[var(--module-accent-ring)]">
                  <span className="w-2 h-2 rounded-full bg-[var(--module-accent)] animate-pulse" />
                  {t("rtsp.runningCount", { count: runningCount })}
                </span>
              )}
            </h3>
            <p className="text-body text-slate-400 mt-0.5">
              {t("rtsp.subtitle")}
            </p>
          </div>
        </div>

        <div className="flex shrink-0 flex-wrap items-center gap-2 self-start md:self-auto whitespace-nowrap">
          <button
            onClick={handleAddInstance}
            className={btnPrimary}
          >
            <Plus className="w-4 h-4" />
            {t("rtsp.addInstance")}
          </button>

          {instances.length - runningCount > 0 && (
            <button
              disabled={batchBusy || instances.some((item) => item.actionLoading)}
              onClick={handleStartAll}
              className={btnSecondary}
            >
              <Play className="w-3.5 h-3.5 fill-current" />
              {t("rtsp.startAll", { count: instances.length - runningCount })}
            </button>
          )}

          {runningCount > 0 && (
            <button
              disabled={batchBusy || instances.some((item) => item.actionLoading)}
              onClick={handleStopAll}
              className={btnDanger}
            >
              <Square className="w-3.5 h-3.5 fill-current" />
              {t("rtsp.stopAll", { count: runningCount })}
            </button>
          )}
        </div>
      </div>

      {panelError && <div role="alert" className="ui-note-error p-3">{panelError}</div>}
      <ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />
      {/* 实例卡片列表（支持全面折叠与精简视图） */}
      <div className="min-h-0 flex-1 overflow-y-auto space-y-3 pr-1">
        {instances.map((inst) => {
          const isRunning = inst.status.running;
          const isLocked = isRunning || inst.actionLoading || batchBusy;

          return (
            <div
              key={inst.id}
              className={`glass-panel border rounded-panel transition-all overflow-hidden ${
                isRunning
                  ? "border-[var(--module-accent-ring)] bg-[color-mix(in_srgb,var(--module-accent)_3%,transparent)] shadow-lg shadow-[var(--module-accent-ring)]"
                  : "border-white/10 bg-slate-900/40 hover:border-white/20"
              }`}
            >
              {/* ── 核心摘要栏（始终保留，点击可展开/折叠） ── */}
              <div
                onClick={() => toggleCollapse(inst.id)}
                className="p-4 flex items-center justify-between gap-3 cursor-pointer hover:bg-white/[0.02] transition-colors"
              >
                {/* 左侧：折叠指示箭头 + 实例图标 + 名称与摘要状态 */}
                <div className="flex items-center gap-3 min-w-0 flex-1">
                  <div className="text-slate-400 hover:text-white transition-colors">
                    {inst.collapsed ? <ChevronRight className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}
                  </div>

                  <div
                    className={`p-2 rounded-card flex-shrink-0 ${
                      isRunning
                        ? "bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-[var(--module-accent)] border border-[var(--module-accent-ring)]"
                        : "bg-white/5 text-slate-400 border border-white/10"
                    }`}
                  >
                    <Radio className="w-4 h-4" />
                  </div>

                  {/* 名称与关键摘要 */}
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2.5">
                      <span className="text-title font-bold text-white truncate">{inst.title}</span>

                      {/* 运行状态 Tag */}
                      {isRunning ? (
                        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-tiny font-semibold bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] flex-shrink-0">
                          <span className="w-1.5 h-1.5 rounded-full bg-[var(--module-accent)] animate-pulse" />
                          {t("rtsp.runningPort", { port: inst.config.port })}
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-tiny font-medium bg-white/5 text-slate-400 border border-white/10 flex-shrink-0">
                          {t("rtsp.stoppedPort", { port: inst.config.port })}
                        </span>
                      )}

                      {/* 视频源摘要 */}
                      <span className="text-tiny text-slate-400 bg-white/5 px-2 py-0.5 rounded border border-white/5 flex-shrink-0 hidden sm:inline-block">
                        {inst.config.sourceType === "camera"
                          ? t("rtsp.sourceCamera", { name: inst.config.cameraName || t("rtsp.notSpecified") })
                          : inst.config.sourceType === "file"
                          ? t("rtsp.sourceFile", { name: inst.config.filePath ? inst.config.filePath.split(/[/\\]/).pop() : t("rtsp.notSpecified") })
                          : t("rtsp.sourceTestsrc")}
                      </span>
                    </div>

                    {/* 折叠时显示的轻量信息 */}
                    <div className="text-caption text-slate-400 font-mono mt-0.5 truncate flex items-center gap-2">
                      <span>{t("rtsp.pathLabel", { path: inst.config.pathName })}</span>
                      <span>·</span>
                      <span>{t("rtsp.protocolLabel", { proto: inst.config.transport?.toUpperCase() || "TCP" })}</span>
                      <span>·</span>
                      <span>{t("rtsp.codecLabel", { codec: inst.config.videoCodec?.toUpperCase() || "H264" })}</span>
                      {isRunning && inst.status.localUrl && (
                        <>
                          <span>·</span>
                          <span className="text-[var(--module-accent)] font-semibold">{inst.status.localUrl}</span>
                        </>
                      )}
                    </div>
                  </div>
                </div>

                {/* 右侧：快捷操作按钮 (启动/停止/删除) */}
                <div className="flex items-center gap-2 flex-shrink-0">
                  {!isRunning ? (
                    <button
                      onClick={(e) => handleStartInstance(inst, e)}
                      disabled={inst.actionLoading || batchBusy}
                      className={btnPrimary}
                    >
                      <Play className="w-3.5 h-3.5 fill-current" />
                      {t("rtsp.startStream")}
                    </button>
                  ) : (
                    <button
                      onClick={(e) => handleStopInstance(inst.id, e)}
                      disabled={inst.actionLoading || batchBusy}
                      className={btnDanger}
                    >
                      <Square className="w-3.5 h-3.5 fill-current" />
                      {t("rtsp.stopInstance")}
                    </button>
                  )}

                  {instances.length > 1 && !isRunning && (
                    <button
                      disabled={inst.actionLoading || batchBusy}
                      onClick={(e) => handleDeleteInstance(inst.id, e)}
                      className="ui-icon-btn w-8 h-8 text-rose-400"
                      title={t("rtsp.deleteInstance")}
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  )}
                </div>
              </div>

              {/* ── 展开区域（仅当 !collapsed 时显示） ── */}
              {!inst.collapsed && (
                <div className="p-5 pt-0 space-y-4 border-t border-white/5">
                  {/* 编辑名称区 */}
                  <div className="flex items-center gap-2 pt-3">
                    <span className="text-body text-slate-400 font-semibold flex-shrink-0">{t("rtsp.aliasLabel")}</span>
                    <input
                      value={inst.title}
                      onChange={(e) => updateInstanceConfig(inst.id, { title: e.target.value })}
                      disabled={isRunning}
                      placeholder={t("rtsp.aliasPh")}
                      className="text-body font-bold text-white bg-white/5 border border-white/10 rounded-ctl px-2.5 py-1 focus:border-[var(--module-accent)] focus:outline-none transition-all flex-1"
                    />
                    <button
                      onClick={(e) => { e.stopPropagation(); handleCopyConfigToAll(inst); }}
                      disabled={instances.length < 2 || batchBusy || instances.some((item) => item.actionLoading)}
                      title={instances.length < 2 ? t("rtsp.copyTitle") : t("rtsp.copyTitleN", { count: instances.length - 1 })}
                      className="px-2.5 py-1 rounded-ctl ui-btn text-slate-300 hover:text-white text-caption font-semibold flex items-center gap-1.5 transition-all cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed flex-shrink-0"
                    >
                      <Copy className="w-3.5 h-3.5" />
                      {t("rtsp.copyToAll", { count: instances.length - 1 })}
                    </button>
                  </div>

                  {/* 运行中推流状态与 RTSP 链接复制区 */}
                  {isRunning && (
                    <div className="rounded-card bg-black/40 border border-[var(--module-accent-ring)] p-4 space-y-3">
                      <div className="flex flex-wrap items-center justify-between text-body font-semibold text-[var(--module-accent)] gap-2 border-b border-white/5 pb-2">
                        <div className="flex items-center gap-2">
                          <span className="w-2.5 h-2.5 rounded-full bg-emerald-400 animate-ping" />
                          {t("rtsp.streamRunning", { mtx: inst.status.mtxPid ?? "N/A", ff: inst.status.ffmpegPid ?? "N/A" })}
                        </div>
                        <div className="text-caption text-slate-400 font-mono">
                          {t("rtsp.uptimeLabel", { time: formatUptime(inst.status.uptimeSeconds) })}
                        </div>
                      </div>

                      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                        {inst.status.localUrl && (
                          <div className="bg-white/[0.03] border border-white/10 rounded-card p-3 flex items-center justify-between">
                            <div className="min-w-0 pr-2">
                              <div className="text-tiny text-slate-400 font-semibold uppercase">{t("rtsp.loopbackLabel")}</div>
                              <div className="text-body font-mono text-[var(--module-accent)] truncate mt-0.5">{inst.status.localUrl}</div>
                            </div>
                            <button
                              onClick={(e) => handleCopy(inst.status.localUrl!, e)}
                              className="p-2 text-slate-300 hover:text-white bg-white/5 hover:bg-white/10 rounded-ctl transition-colors cursor-pointer flex-shrink-0"
                              title={t("rtsp.copyRtsp")}
                            >
                              {copiedUrl === inst.status.localUrl ? <CheckCircle className="w-4 h-4 text-[var(--module-accent)]" /> : <Copy className="w-4 h-4" />}
                            </button>
                          </div>
                        )}
                        {inst.config.allowLan && allIps.length > 0 && (
                          <div className="md:col-span-2 bg-white/[0.03] border border-white/10 rounded-card p-3 space-y-2">
                            <div className="flex items-center justify-between">
                              <div className="text-tiny text-slate-400 font-semibold uppercase">{t("rtsp.lanLabel")}</div>
                              <button
                                onClick={loadAllIps}
                                className="text-tiny text-slate-400 hover:text-white flex items-center gap-0.5 cursor-pointer"
                              >
                                <RefreshCw className="w-3 h-3" /> {t("rtsp.refresh")}
                              </button>
                            </div>
                            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                              {allIps.map(({ name, ip }) => {
                                const url = `rtsp://${ip}:${inst.config.port}/${inst.config.pathName}`;
                                return (
                                  <div
                                    key={ip}
                                    onClick={(e) => handleCopy(url, e)}
                                    className="flex items-center justify-between bg-black/30 border border-white/10 rounded-ctl px-2.5 py-1.5 cursor-pointer hover:bg-white/[0.06] transition-colors group"
                                  >
                                    <div className="min-w-0 pr-2">
                                      <div className="text-tiny text-slate-500 truncate">{name}</div>
                                      <div className="text-caption font-mono text-cyan-400 truncate">{url}</div>
                                    </div>
                                    <div className="flex-shrink-0">
                                      {copiedUrl === url ? (
                                        <CheckCircle className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                                      ) : (
                                        <Copy className="w-3.5 h-3.5 text-slate-500 group-hover:text-white transition-colors" />
                                      )}
                                    </div>
                                  </div>
                                );
                              })}
                            </div>
                          </div>
                        )}
                      </div>
                    </div>
                  )}

                  {/* 异常报错提示 */}
                  {inst.error && (
                    <div role="alert" className="p-3.5 rounded-card bg-rose-500/10 border border-rose-500/20 text-body text-rose-300 flex items-start gap-2.5">
                      <AlertCircle className="w-4 h-4 flex-shrink-0 mt-0.5" />
                      <div className="min-w-0 whitespace-pre-wrap font-mono text-caption leading-relaxed">{inst.error}</div>
                    </div>
                  )}

                  {/* 3 卡片配置区 */}
                  <div className="grid grid-cols-1 lg:grid-cols-3 gap-4 items-start">
                    {/* ── 卡片 1：视频输入源 ── */}
                    <div className="rounded-panel border border-white/10 bg-white/[0.02] overflow-hidden">
                      <div className="px-4 py-2.5 border-b border-white/5 bg-white/[0.02] flex items-center gap-2">
                        <div className="p-1.5 rounded-ctl bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)]">
                          <Sliders className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                        </div>
                        <span className="text-body font-semibold text-white">{t("rtsp.sourceTitle")}</span>
                      </div>
                      <div className="p-4 space-y-3">
                        <div className="grid grid-cols-3 gap-1.5">
                          {(["testsrc", "camera", "file"] as const).map((st) => (
                            <button
                              key={st}
                              disabled={isLocked}
                              onClick={() => updateInstanceConfig(inst.id, { sourceType: st })}
                              className={`h-9 text-body font-medium rounded-card border transition-all cursor-pointer ${
                                inst.config.sourceType === st
                                  ? "bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] border-[color-mix(in_srgb,var(--module-accent)_50%,transparent)] text-[var(--module-accent)] font-semibold shadow-sm"
                                  : "bg-white/5 border-white/10 text-slate-400 hover:text-white"
                              }`}
                            >
                              {st === "testsrc" ? t("rtsp.sourceTestsrcOpt") : st === "camera" ? t("rtsp.sourceCameraOpt") : t("rtsp.sourceFileOpt")}
                            </button>
                          ))}
                        </div>

                        {inst.config.sourceType === "camera" && (
                          <>
                            <div>
                              <div className="flex items-center justify-between text-tiny text-slate-400 mb-1">
                                <span>{t("rtsp.selectCamera")}</span>
                                <button
                                  onClick={fetchDevices}
                                  disabled={loadingDevices || isLocked}
                                  className="hover:text-white flex items-center gap-0.5 cursor-pointer"
                                >
                                  <RefreshCw className={`w-3 h-3 ${loadingDevices ? "animate-spin" : ""}`} /> {t("rtsp.refresh")}
                                </button>
                              </div>
                              <select
                                value={inst.config.cameraName || ""}
                                disabled={isLocked}
                                onChange={(e) => updateInstanceConfig(inst.id, { cameraName: e.target.value })}
                                className="w-full h-9 px-2.5 rounded-card ui-input text-body text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer"
                              >
                                {devices.videoDevices.length === 0 ? (
                                  <option value="">{t("rtsp.noCameraFound")}</option>
                                ) : (
                                  devices.videoDevices.map((d) => (
                                    <option key={d} value={d}>
                                      {d}
                                    </option>
                                  ))
                                )}
                              </select>
                            </div>

                          </>
                        )}

                        {inst.config.sourceType === "file" && (
                          <div className="space-y-1.5">
                            <div className="flex items-center gap-1.5">
                              <input
                                value={inst.config.filePath || ""}
                                disabled={isLocked}
                                onChange={(e) => updateInstanceConfig(inst.id, { filePath: e.target.value })}
                                placeholder={t("rtsp.filePh")}
                                className="flex-1 h-9 px-2.5 rounded-card bg-white/5 border border-white/10 text-body text-white placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent)]"
                              />
                              <button
                                disabled={isLocked}
                                onClick={() => handleSelectFile(inst.id)}
                                className="h-9 px-3 rounded-card bg-white/10 hover:bg-white/20 text-white transition-colors cursor-pointer flex items-center justify-center"
                                title={t("rtsp.pickFile")}
                              >
                                <FolderOpen className="w-4 h-4" />
                              </button>
                            </div>
                            <label className="flex items-center gap-1.5 text-caption text-slate-400 cursor-pointer">
                              <input
                                type="checkbox"
                                disabled={isLocked}
                                checked={inst.config.loopFile}
                                onChange={(e) => updateInstanceConfig(inst.id, { loopFile: e.target.checked })}
                                className="rounded bg-white/5 border-white/20 text-[var(--module-accent)] focus:ring-0"
                              />
                              {t("rtsp.loopVideo")}
                            </label>
                          </div>
                        )}

                        {/* 音频设置：摄像头模式可选麦克风；测试画幅模式推送 1kHz 合成测试音 */}
                        {(inst.config.sourceType === "camera" || inst.config.sourceType === "testsrc") && (
                          <div className="rounded-card border border-white/10 bg-white/[0.02] p-3 space-y-2">
                            <label className="flex items-center justify-between cursor-pointer">
                              <span className="text-caption text-slate-300 flex items-center gap-1.5">
                                {inst.config.includeAudio ? (
                                  <Mic className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                                ) : (
                                  <MicOff className="w-3.5 h-3.5 text-slate-500" />
                                )}
                                {t("rtsp.enableAudio")}
                              </span>
                              <button
                                type="button"
                                role="switch"
                                aria-checked={inst.config.includeAudio}
                                disabled={isLocked}
                                onClick={() =>
                                  updateInstanceConfig(inst.id, { includeAudio: !inst.config.includeAudio })
                                }
                                className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors cursor-pointer ${
                                  inst.config.includeAudio ? "bg-[var(--module-accent)]" : "bg-white/15"
                                } ${isLocked ? "opacity-50 cursor-not-allowed" : ""}`}
                              >
                                <span
                                  className={`inline-block h-3.5 w-3.5 rounded-full bg-white shadow-sm transition-transform ${
                                    inst.config.includeAudio ? "translate-x-4" : "translate-x-0.5"
                                  }`}
                                />
                              </button>
                            </label>

                            {inst.config.includeAudio && inst.config.sourceType === "camera" && (
                              <div>
                                <span className="text-tiny text-slate-400 block mb-1">{t("rtsp.selectMic")}</span>
                                <select
                                  value={inst.config.audioDevice || ""}
                                  disabled={isLocked}
                                  onChange={(e) => updateInstanceConfig(inst.id, { audioDevice: e.target.value })}
                                  className="w-full h-9 px-2.5 rounded-card ui-input text-body text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer"
                                >
                                  {devices.audioDevices.length === 0 ? (
                                    <option value="">{t("rtsp.noMicFound")}</option>
                                  ) : (
                                    devices.audioDevices.map((d) => (
                                      <option key={d} value={d}>
                                        {d}
                                      </option>
                                    ))
                                  )}
                                </select>
                                <p className="text-tiny text-slate-500 mt-1 leading-relaxed">
                                  {t("rtsp.audioHint")}
                                </p>
                              </div>
                            )}

                            {inst.config.includeAudio && inst.config.sourceType === "testsrc" && (
                              <div>
                                <span className="text-tiny text-slate-400 block mb-1">{t("rtsp.testAudioLabel")}</span>
                                <select
                                  value={inst.config.testAudioType || "tone1000"}
                                  disabled={isLocked}
                                  onChange={(e) => updateInstanceConfig(inst.id, { testAudioType: e.target.value })}
                                  className="w-full h-9 px-2.5 rounded-card ui-input text-body text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer"
                                >
                                  <option value="tone1000">{t("rtsp.testAudioTone1000")}</option>
                                  <option value="tone440">{t("rtsp.testAudioTone440")}</option>
                                  <option value="stereo">{t("rtsp.testAudioStereo")}</option>
                                  <option value="sweep">{t("rtsp.testAudioSweep")}</option>
                                  <option value="noise">{t("rtsp.testAudioNoise")}</option>
                                  <option value="beep">{t("rtsp.testAudioBeep")}</option>
                                </select>
                                <p className="text-tiny text-slate-500 mt-1 leading-relaxed">
                                  {t("rtsp.testsrcAudioHint")}
                                </p>
                              </div>
                            )}
                          </div>
                        )}

                        {inst.config.sourceType === "testsrc" && (
                          <div className="text-caption text-slate-400 bg-white/[0.02] border border-white/5 rounded-card p-2.5">
                            {t("rtsp.testsrcDesc")}
                          </div>
                        )}
                      </div>
                    </div>

                    {/* ── 卡片 2：网络与传输协议 ── */}
                    <div className="rounded-panel border border-white/10 bg-white/[0.02] overflow-hidden">
                      <div className="px-4 py-2.5 border-b border-white/5 bg-white/[0.02] flex items-center gap-2">
                        <div className="p-1.5 rounded-ctl bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)]">
                          <Network className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                        </div>
                        <span className="text-body font-semibold text-white">{t("rtsp.netTitle")}</span>
                      </div>
                      <div className="p-4 space-y-3">
                        <div className="grid grid-cols-2 gap-2">
                          <div>
                            <span className="text-tiny text-slate-400 block mb-1">{t("rtsp.portLabel")}</span>
                            <input
                              type="number"
                              disabled={isLocked}
                              value={inst.config.port || ""}
                              onChange={(e) =>
                                updateInstanceConfig(inst.id, { port: e.target.value === "" ? 0 : Number(e.target.value) })
                              }
                              className="w-full h-9 px-2.5 rounded-card bg-white/5 border border-white/10 text-body text-white focus:outline-none focus:border-[var(--module-accent)] font-mono"
                            />
                          </div>
                          <div>
                            <span className="text-tiny text-slate-400 block mb-1">{t("rtsp.pathNameLabel")}</span>
                            <input
                              disabled={isLocked}
                              value={inst.config.pathName}
                              onChange={(e) => updateInstanceConfig(inst.id, { pathName: e.target.value })}
                              className="w-full h-9 px-2.5 rounded-card bg-white/5 border border-white/10 text-body text-white focus:outline-none focus:border-[var(--module-accent)] font-mono"
                            />
                          </div>
                        </div>

                        <div>
                          <span className="text-tiny text-slate-400 block mb-1">{t("rtsp.transportLabel")}</span>
                          <select
                            disabled={isLocked}
                            value={inst.config.transport || "tcp"}
                            onChange={(e) =>
                              updateInstanceConfig(inst.id, { transport: e.target.value as "tcp" | "udp" })
                            }
                            className="w-full h-9 px-2.5 rounded-card ui-input text-body text-white focus:outline-none focus:border-[var(--module-accent)] cursor-pointer"
                          >
                            <option value="tcp">{t("rtsp.optTcp")}</option>
                            <option value="udp">{t("rtsp.optUdp")}</option>
                          </select>
                        </div>

                        <div>
                          <label className="flex items-center gap-1.5 text-caption text-slate-400 cursor-pointer">
                            <input
                              type="checkbox"
                              disabled={isLocked}
                              checked={inst.config.allowLan}
                              onChange={(e) => updateInstanceConfig(inst.id, { allowLan: e.target.checked })}
                              className="rounded bg-white/5 border-white/20 text-[var(--module-accent)] focus:ring-0"
                            />
                            {t("rtsp.allowLan")}
                          </label>

                          {inst.config.allowLan && (
                            <div className="mt-2 space-y-1.5">
                              <div className="flex items-center justify-between">
                                <span className="text-tiny text-slate-500">{t("rtsp.nicIpHint")}</span>
                                <button
                                  onClick={loadAllIps}
                                  className="text-tiny text-slate-400 hover:text-white flex items-center gap-0.5 cursor-pointer"
                                >
                                  <RefreshCw className="w-3 h-3" /> {t("rtsp.refresh")}
                                </button>
                              </div>
                              {allIps.length === 0 ? (
                                <div className="text-tiny text-slate-500 bg-white/[0.02] border border-white/5 rounded-ctl px-2.5 py-1.5">
                                  {t("rtsp.noNicIp")}
                                </div>
                              ) : (
                                allIps.map(({ name, ip }) => {
                                  const url = `rtsp://${ip}:${inst.config.port}/${inst.config.pathName}`;
                                  return (
                                    <div
                                      key={ip}
                                      onClick={() => handleCopy(url)}
                                      className="flex items-center justify-between bg-white/[0.03] border border-white/10 rounded-ctl px-2.5 py-1.5 cursor-pointer hover:bg-white/[0.06] transition-colors group"
                                    >
                                      <div className="min-w-0">
                                        <div className="text-tiny text-slate-500 truncate">{name}</div>
                                        <div className="text-caption font-mono text-cyan-400 truncate">{url}</div>
                                      </div>
                                      <div className="flex-shrink-0 ml-2">
                                        {copiedUrl === url ? (
                                          <CheckCircle className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                                        ) : (
                                          <Copy className="w-3.5 h-3.5 text-slate-500 group-hover:text-white transition-colors" />
                                        )}
                                      </div>
                                    </div>
                                  );
                                })
                              )}
                            </div>
                          )}
                        </div>
                      </div>
                    </div>

                    {/* ── 卡片 3：画幅与编码选项 ── */}
                    <RtspEncodingOptions
                      isLocked={isLocked}
                      inst={inst}
                      updateInstanceConfig={updateInstanceConfig}
                    />
                  </div>

                  {/* 日志查看区 */}
                  {inst.status.logs && inst.status.logs.length > 0 && (
                    <div className="pt-2 border-t border-white/5">
                      <button
                        onClick={() =>
                          setInstances((prev) =>
                            prev.map((i) => (i.id === inst.id ? { ...i, showLogs: !i.showLogs } : i))
                          )
                        }
                        className="text-caption text-slate-400 hover:text-white flex items-center gap-1.5 cursor-pointer"
                      >
                        <Terminal className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                        <span>{t("rtsp.viewLogs", { count: inst.status.logs.length })}</span>
                        {inst.showLogs ? <ChevronUp className="w-3.5 h-3.5" /> : <ChevronDown className="w-3.5 h-3.5" />}
                      </button>

                      {inst.showLogs && (
                        <pre className="mt-2 text-tiny text-slate-300 bg-black/60 border border-white/10 rounded-card p-3.5 max-h-52 overflow-y-auto font-mono whitespace-pre-wrap leading-relaxed">
                          {inst.status.logs.join("\n")}
                        </pre>
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
