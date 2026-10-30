import { type OtpToken } from "./OtpPanel.model";
// 品牌图标：根据 customIcon / issuer 显示品牌标识（字母 + 主题色块）。
// 电脑端不内嵌 logo 图片资源，用「品牌名首字母 + 哈希色」作为图标，简洁且无需资源。
export function BrandIcon({ token }: { token: OtpToken }) {
  const label = token.customIcon || token.issuer || token.account || "?";
  const initial = (label.trim()[0] || "?").toUpperCase();
  // 根据 issuer 生成稳定的哈希色
  let hash = 0;
  const src = token.issuer || token.account;
  for (let i = 0; i < src.length; i++) hash = (hash * 31 + src.charCodeAt(i)) | 0;
  const hue = Math.abs(hash) % 360;
  return (
    <div
      className="w-8 h-8 rounded-ctl flex items-center justify-center text-title font-bold text-white flex-shrink-0"
      style={{ background: `linear-gradient(135deg, hsl(${hue},65%,45%), hsl(${(hue + 40) % 360},65%,35%))` }}
    >
      {initial}
    </div>
  );
}
