import { useState, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { 
  Upload, 
  Image as ImageIcon, 
  Copy, 
  Download, 

  FileCode,
  CheckCircle,
  AlertCircle,
  Scissors
} from "lucide-react";

export default function ImageBase64() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<"toBase64" | "toImage">("toBase64");
  
  // Image to Base64 State
  const [imgSrc, setImgSrc] = useState<string | null>(null);
  const [base64Result, setBase64Result] = useState("");
  const [imgDetails, setImgDetails] = useState<{ name: string; size: string; type: string } | null>(null);
  const [copySuccess, setCopySuccess] = useState<"base64" | "html" | "css" | null>(null);
  const [error1, setError1] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Base64 to Image State
  const [base64Input, setBase64Input] = useState("");
  const [previewSrc, setPreviewSrc] = useState<string | null>(null);
  const [previewDetails, setPreviewDetails] = useState<{ type: string; size: string } | null>(null);
  const [error2, setError2] = useState<string | null>(null);
  const [saveLoading, setSaveLoading] = useState(false);

  // --- Image to Base64 handlers ---
  const handleSelectLocalImage = async () => {
    setError1(null);
    try {
      const selected = await open({
        multiple: false,
        filters: [{
          name: "Images",
          extensions: ["png", "jpg", "jpeg", "gif", "webp", "svg", "ico"]
        }]
      });
      if (selected && typeof selected === "string") {
        const b64 = await invoke<string>("image_to_base64", { filePath: selected });
        setImgSrc(b64);
        setBase64Result(b64);
        
        // Extract meta details
        const fileName = selected.substring(selected.lastIndexOf("\\") + 1);
        const match = b64.match(/^data:(image\/[a-z0-9+.-]+);base64,/);
        const type = match ? match[1] : "image/png";
        
        // Estimate size from Base64 length
        const base64Len = b64.substring(b64.indexOf(",") + 1).length;
        const sizeBytes = Math.floor(base64Len * 0.75);
        setImgDetails({
          name: fileName,
          size: (sizeBytes / 1024).toFixed(1) + " KB",
          type
        });
      }
    } catch (e: any) {
      setError1(String(e));
    }
  };

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    
    const reader = new FileReader();
    reader.onload = (event) => {
      const result = event.target?.result;
      if (typeof result === "string") {
        setImgSrc(result);
        setBase64Result(result);
        setImgDetails({
          name: file.name,
          size: (file.size / 1024).toFixed(1) + " KB",
          type: file.type
        });
      }
    };
    reader.readAsDataURL(file);
  };

  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault();
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    const file = e.dataTransfer.files?.[0];
    if (file && file.type.startsWith("image/")) {
      const reader = new FileReader();
      reader.onload = (event) => {
        const result = event.target?.result;
        if (typeof result === "string") {
          setImgSrc(result);
          setBase64Result(result);
          setImgDetails({
            name: file.name,
            size: (file.size / 1024).toFixed(1) + " KB",
            type: file.type
          });
        }
      };
      reader.readAsDataURL(file);
    }
  };

  const handleCopy = (type: "base64" | "html" | "css") => {
    let text = base64Result;
    if (type === "html") {
      text = `<img src="${base64Result}" alt="image" />`;
    } else if (type === "css") {
      text = `background-image: url("${base64Result}");`;
    }
    navigator.clipboard.writeText(text);
    setCopySuccess(type);
    setTimeout(() => setCopySuccess(null), 2000);
  };

  const handleClearImage = () => {
    setImgSrc(null);
    setBase64Result("");
    setImgDetails(null);
    setError1(null);
    if (fileInputRef.current) fileInputRef.current.value = "";
  };

  // --- Base64 to Image handlers ---
  const handleBase64InputChange = (val: string) => {
    setBase64Input(val);
    setError2(null);
    
    if (!val.trim()) {
      setPreviewSrc(null);
      setPreviewDetails(null);
      return;
    }

    // Auto-detect format & try rendering
    let formatted = val.trim();
    if (!formatted.startsWith("data:image/")) {
      // Find standard headers if they pasted raw base64 but we need to guess format
      // Default to png if raw
      formatted = `data:image/png;base64,${formatted}`;
    }

    // Basic Base64 Validation
    const base64Data = formatted.substring(formatted.indexOf(",") + 1);
    const regex = /^[a-zA-Z0-9+/]*={0,2}$/;
    if (!regex.test(base64Data.replace(/\s/g, ""))) {
      setError2(t("imgbase64.invalidBase64"));
      setPreviewSrc(null);
      setPreviewDetails(null);
      return;
    }

    setPreviewSrc(formatted);
    const match = formatted.match(/^data:(image\/[a-z0-9+.-]+);base64,/);
    const type = match ? match[1] : "image/png";
    const sizeBytes = Math.floor(base64Data.length * 0.75);
    setPreviewDetails({
      type,
      size: (sizeBytes / 1024).toFixed(1) + " KB"
    });
  };

  const handleSaveImage = async () => {
    if (!previewSrc) return;
    setSaveLoading(true);
    try {
      const ext = previewDetails?.type.split("/")[1] || "png";
      const savePath = await save({
        title: t("imgbase64.saveTitle"),
        defaultPath: `image.${ext}`,
        filters: [{
          name: "Image",
          extensions: [ext]
        }]
      });
      
      if (savePath) {
        await invoke("save_base64_image", {
          base64Str: previewSrc,
          filePath: savePath
        });
        alert(t("imgbase64.saveOk"));
      }
    } catch (e: any) {
      alert(t("imgbase64.saveFail", { err: String(e) }));
    } finally {
      setSaveLoading(false);
    }
  };

  return (
    <div className="space-y-6 px-20 py-4 ">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-semibold text-white">{t("imgbase64.title")}</h3>
          <p className="text-[11px] text-slate-400 mt-0.5">{t("imgbase64.subtitle")}</p>
        </div>
        
        {/* Tab Selector */}
        <div className="flex bg-white/5 border border-white/5 rounded-lg p-0.5">
          <button
            onClick={() => setActiveTab("toBase64")}
            className={`px-3 py-1 rounded text-[10px] font-semibold transition-all cursor-pointer ${
              activeTab === "toBase64" ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200"
            }`}
          >
            {t("imgbase64.imgToBase64")}
          </button>
          <button
            onClick={() => setActiveTab("toImage")}
            className={`px-3 py-1 rounded text-[10px] font-semibold transition-all cursor-pointer ${
              activeTab === "toImage" ? "bg-blue-600 text-white" : "text-slate-400 hover:text-slate-200"
            }`}
          >
            {t("imgbase64.base64ToImg")}
          </button>
        </div>
      </div>

      {/* 图片转 Base64 面板 */}
      {activeTab === "toBase64" && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-5">
          {/* 左侧：图片上传与预览 */}
          <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 flex flex-col h-[400px]">
            <div className="flex items-center justify-between border-b border-white/5 pb-3 mb-4 flex-shrink-0">
              <span className="text-xs font-semibold text-white flex items-center gap-1.5">
                <ImageIcon className="w-4 h-4 text-blue-400" />
                {t("imgbase64.imgSource")}
              </span>
              {imgSrc && (
                <button
                  onClick={handleClearImage}
                  className="text-[10px] text-red-400 hover:text-red-300 font-semibold cursor-pointer"
                >
                  {t("imgbase64.clearImg")}
                </button>
              )}
            </div>

            {!imgSrc ? (
              <div 
                onDragOver={handleDragOver}
                onDrop={handleDrop}
                className="flex-1 border-2 border-dashed border-white/10 hover:border-blue-500/50 rounded-2xl flex flex-col items-center justify-center p-6 text-center space-y-3 cursor-pointer transition-colors bg-black/10"
                onClick={() => fileInputRef.current?.click()}
              >
                <div className="w-12 h-12 rounded-full bg-blue-600/10 flex items-center justify-center">
                  <Upload className="w-6 h-6 text-blue-400" />
                </div>
                <div>
                  <p className="text-xs text-slate-300 font-medium">{t("imgbase64.dragHint")}</p>
                  <p className="text-[10px] text-slate-500 mt-1">{t("imgbase64.formatsHint")}</p>
                </div>
                <div className="pt-2">
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      handleSelectLocalImage();
                    }}
                    className="px-3.5 py-1.5 bg-white/5 hover:bg-white/10 border border-white/5 text-slate-300 rounded-lg text-[10px] font-semibold cursor-pointer transition-colors"
                  >
                    {t("imgbase64.pickFromDir")}
                  </button>
                </div>
                <input
                  type="file"
                  ref={fileInputRef}
                  onChange={handleFileChange}
                  accept="image/*"
                  className="hidden"
                />
              </div>
            ) : (
              <div className="flex-1 flex flex-col min-h-0 bg-black/20 rounded-xl overflow-hidden border border-white/5">
                <div className="flex-1 p-4 flex items-center justify-center min-h-0">
                  <img
                    src={imgSrc}
                    alt="Preview"
                    className="max-h-full max-w-full object-contain rounded shadow-lg"
                  />
                </div>
                {imgDetails && (
                  <div className="bg-black/40 px-4 py-2 border-t border-white/5 flex items-center justify-between text-[10px] text-slate-400 font-mono flex-shrink-0">
                    <span className="truncate max-w-[200px]">{imgDetails.name}</span>
                    <span>{imgDetails.type} • {imgDetails.size}</span>
                  </div>
                )}
              </div>
            )}

            {error1 && (
              <div className="mt-3 p-3 bg-red-500/10 border border-red-500/20 text-red-200 text-xs rounded-xl flex items-center gap-2 flex-shrink-0">
                <AlertCircle className="w-4 h-4 text-red-400" />
                <span>{error1}</span>
              </div>
            )}
          </div>

          {/* 右侧：Base64 结果 */}
          <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 flex flex-col h-[400px]">
            <div className="flex items-center justify-between border-b border-white/5 pb-3 mb-4 flex-shrink-0">
              <span className="text-xs font-semibold text-white flex items-center gap-1.5">
                <FileCode className="w-4 h-4 text-blue-400" />
                {t("imgbase64.base64Result")}
              </span>
            </div>

            <div className="flex-1 min-h-0 relative">
              <textarea
                value={base64Result}
                readOnly
                placeholder={t("imgbase64.resultPh")}
                className="w-full h-full glass-input p-4 font-mono text-[10px] text-slate-300 resize-none break-all"
              />
              {base64Result && (
                <div className="absolute top-2 right-2 flex items-center gap-1.5">
                  <span className="text-[9px] text-slate-500 bg-black/40 px-2 py-1 rounded border border-white/5 font-mono">
                    {t("imgbase64.length", { n: base64Result.length })}
                  </span>
                </div>
              )}
            </div>

            {base64Result && (
              <div className="mt-4 flex flex-wrap gap-2 flex-shrink-0">
                <button
                  onClick={() => handleCopy("base64")}
                  className="flex-1 px-3 py-2 bg-blue-600 hover:bg-blue-500 text-white rounded-xl text-[10px] font-semibold cursor-pointer transition-all flex items-center justify-center gap-1.5"
                >
                  {copySuccess === "base64" ? <CheckCircle className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
                  {copySuccess === "base64" ? t("imgbase64.copied") : t("imgbase64.copyBase64")}
                </button>
                <button
                  onClick={() => handleCopy("html")}
                  className="px-3 py-2 bg-white/5 hover:bg-white/10 text-slate-300 border border-white/5 rounded-xl text-[10px] font-semibold cursor-pointer transition-colors flex items-center gap-1"
                >
                  {copySuccess === "html" ? <CheckCircle className="w-3.5 h-3.5 text-emerald-400" /> : <FileCode className="w-3.5 h-3.5" />}
                  {t("imgbase64.htmlTag")}
                </button>
                <button
                  onClick={() => handleCopy("css")}
                  className="px-3 py-2 bg-white/5 hover:bg-white/10 text-slate-300 border border-white/5 rounded-xl text-[10px] font-semibold cursor-pointer transition-colors flex items-center gap-1"
                >
                  {copySuccess === "css" ? <CheckCircle className="w-3.5 h-3.5 text-emerald-400" /> : <Scissors className="w-3.5 h-3.5" />}
                  {t("imgbase64.cssStyle")}
                </button>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Base64 转图片面板 */}
      {activeTab === "toImage" && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-5">
          {/* 左侧：输入框 */}
          <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 flex flex-col h-[400px]">
            <div className="flex items-center justify-between border-b border-white/5 pb-3 mb-4 flex-shrink-0">
              <span className="text-xs font-semibold text-white flex items-center gap-1.5">
                <FileCode className="w-4 h-4 text-blue-400" />
                {t("imgbase64.inputBase64")}
              </span>
              {base64Input && (
                <button
                  onClick={() => handleBase64InputChange("")}
                  className="text-[10px] text-red-400 hover:text-red-300 font-semibold cursor-pointer"
                >
                  {t("imgbase64.clear")}
                </button>
              )}
            </div>

            <div className="flex-1 min-h-0">
              <textarea
                value={base64Input}
                onChange={(e) => handleBase64InputChange(e.target.value)}
                placeholder={t("imgbase64.inputPh")}
                className="w-full h-full glass-input p-4 font-mono text-[10px] text-slate-300 resize-none break-all"
              />
            </div>

            {error2 && (
              <div className="mt-3 p-3 bg-red-500/10 border border-red-500/20 text-red-200 text-xs rounded-xl flex items-center gap-2 flex-shrink-0">
                <AlertCircle className="w-4 h-4 text-red-400" />
                <span>{error2}</span>
              </div>
            )}
          </div>

          {/* 右侧：图片预览及保存 */}
          <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 flex flex-col h-[400px]">
            <div className="flex items-center justify-between border-b border-white/5 pb-3 mb-4 flex-shrink-0">
              <span className="text-xs font-semibold text-white flex items-center gap-1.5">
                <ImageIcon className="w-4 h-4 text-blue-400" />
                {t("imgbase64.preview")}
              </span>
            </div>

            {!previewSrc ? (
              <div className="flex-1 border border-white/5 rounded-xl bg-black/10 flex flex-col items-center justify-center p-6 text-center text-slate-500 text-xs">
                {t("imgbase64.previewHint")}
              </div>
            ) : (
              <div className="flex-1 flex flex-col min-h-0 bg-black/20 border border-white/5 rounded-xl overflow-hidden">
                <div className="flex-1 p-4 flex items-center justify-center min-h-0">
                  <img
                    src={previewSrc}
                    alt="Restored Preview"
                    onError={() => setError2(t("imgbase64.loadFail"))}
                    className="max-h-full max-w-full object-contain rounded shadow-lg"
                  />
                </div>
                {previewDetails && (
                  <div className="bg-black/40 px-4 py-2 border-t border-white/5 flex items-center justify-between text-[10px] text-slate-400 font-mono flex-shrink-0">
                    <span>{t("imgbase64.detectFormat", { type: previewDetails.type })}</span>
                    <span>{t("imgbase64.fileSize", { size: previewDetails.size })}</span>
                  </div>
                )}
              </div>
            )}

            {previewSrc && (
              <div className="mt-4 flex-shrink-0">
                <button
                  onClick={handleSaveImage}
                  disabled={saveLoading}
                  className="w-full px-5 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/20 cursor-pointer transition-all flex items-center justify-center gap-1.5"
                >
                  <Download className="w-3.5 h-3.5" />
                  {saveLoading ? t("imgbase64.saving") : t("imgbase64.saveAsImg")}
                </button>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
