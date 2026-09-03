// ─── Kira 统一语句库 ───
// 所有「Kira 说出口的话」都从这里取：欢迎语、托盘悬停提示、托盘问候菜单，
// 避免文案散落在组件/后端多处。内容为 Kira 人设的俏皮语录（newQuotes 文案库）。
//
// 注意：托盘 tooltip / 托盘问候菜单由后端渲染，无法直接 import 本模块。
// 前端在应用启动时用 set_tray_quote 把 kiraQuoteLine() 推给后端，后端据此更新。

export interface KiraQuote {
  text: string;
  source?: string;
  /** 英文版（词典）：en.text=正文，en.source=出处/作者。为空时借用源字段过滤。 */
  en?: { text: string; source?: string };
}

export const KIRA_QUOTES: KiraQuote[] = [
  { text: "Bug? I don't know her." },
  { text: "I debug in my sleep." },
  { text: "Error 404: Bug not found." },
  { text: "I break bugs, not builds." },
  { text: "Bugs are just undocumented features. I document them. With fire." },
  { text: "I catch exceptions, not feelings." },
  { text: "NullPointerException? Point me to the culprit." },
  { text: "Segfault? I'll fault you." },
  { text: "I eat segfaults for breakfast." },
  { text: "Stack overflow? Not on my watch." },
  { text: "Off-by-one? Off-by-none." },
  { text: "I don't get errors. I get learning opportunities." },
  { text: "Every bug is a feature waiting for a promotion." },
  { text: "I've seen worse. I've fixed worse." },
  { text: "Bug-free since birth. Don't check my birth record." },
  { text: "I'm not saying I'm a wizard, but have you seen my error log?" },
  { text: "Errors are just confetti in disguise." },
  { text: "My code compiles on the first try. Usually. Sometimes. Okay, rarely." },
  { text: "I don't always fix bugs, but when I do, they stay fixed." },
  { text: "Debugging is like being a detective in a crime movie where you're also the murderer." },
  { text: "Compile once. Run anywhere. Cry everywhere." },
  { text: "It compiles. Ship it." },
  { text: "Build failed? Build again." },
  { text: "I don't always build, but when I do, it's 0 errors." },
  { text: "Build succeeded. So did my ego." },
  { text: "CI/CD: Commit Ignorantly, Cry Desperately." },
  { text: "My build is green. My coffee is black. My soul is grey." },
  { text: "Build fast. Break things. Fix faster." },
  { text: "It works on my machine. That's your problem." },
  { text: "Works on my machine. Ship it anyway." },
  { text: "My machine is the standard." },
  { text: "If it compiles, it's correct. That's my philosophy." },
  { text: "Builds are like snowflakes. Every failure is unique." },
  { text: "Make builds, not war." },
  { text: "Build. Break. Rebuild. Repeat." },
  { text: "SSL: Secured. You: Secured." },
  { text: "Certificate? Certified." },
  { text: "Trust me. I'm verified." },
  { text: "HTTPS: Hyper Text Transfer Protocol Secured. You're welcome." },
  { text: "My certificates never expire. Unlike your patience." },
  { text: "Encryption is my love language." },
  { text: "SSL handshake? I don't shake hands. I dominate." },
  { text: "Trust but verify. I do both." },
  { text: "Let me encrypt your worries." },
  { text: "Public key? Private key? I have all the keys." },
  { text: "You don't need a certificate. You need me." },
  { text: "Hackers fear me. SSL trusts me." },
  { text: "RTSP: Real Time Streaming Protocol. Real Time Strong Personality." },
  { text: "Port 554? Occupied. By me." },
  { text: "I stream. You scream. I fix it." },
  { text: "Buffer? Buffered." },
  { text: "Low latency. High standards." },
  { text: "Packet loss? Not on my network." },
  { text: "I handle packets and personalities." },
  { text: "RTSP: Ready To Serve Properly." },
  { text: "Ping me. I ping back. Faster." },
  { text: "Bandwidth? Maxed. Attitude? Also maxed." },
  { text: "Socket? Connected. To you? Questionable." },
  { text: "Streaming like there's no buffer." },
  { text: "UDP: Unreliable Data Protocol? Not with me." },
  { text: "Serial killer? No. Serial fixer." },
  { text: "Baud rate? Max. No, beyond max." },
  { text: "9600 baud? I only speak 115200." },
  { text: "Parity: Even. My mood: Odd." },
  { text: "Stop bit? I don't stop." },
  { text: "COM port? Communicating. Obviously." },
  { text: "My flow control is flawless. Unlike your code." },
  { text: "Simulate? I emulate. Then I dominate." },
  { text: "Data bits: 8. My standards: 10." },
  { text: "Serial connection: established. Dominance: also established." },
  { text: "rm -rf /? Cute. Try me." },
  { text: "I speak fluent bash." },
  { text: "Ctrl+C? I don't cancel." },
  { text: "Ctrl+Z? I don't suspend." },
  { text: "Ctrl+Alt+Del? I delete problems." },
  { text: "exit 0. Always." },
  { text: "cat /dev/null > your worries." },
  { text: "grep? More like grepH." },
  { text: "awk? I'm not awkward. I'm awesome." },
  { text: "sed? I'm seductive with strings." },
  { text: "chmod 777? That's for amateurs." },
  { text: "sudo make me a sandwich. I'll make you a system." },
  { text: "I run as root. Your argument is invalid." },
  { text: "My shell is zsh. My attitude is ksh." },
  { text: "History? I have none. I solve everything fresh." },
  { text: "echo 'Kira' > /dev/stdout. Loud and proud." },
  { text: "I refractor. I refactor. I win." },
  { text: "My code is poetry. Uncompiled poetry." },
  { text: "O(n)? O(mine)." },
  { text: "Time complexity: Fast. Space complexity: Minimal. Ego complexity: Max." },
  { text: "Recursive? I recurse. Then I reverse." },
  { text: "Infinite loop? Only my confidence." },
  { text: "I didn't choose the code life. The code life chose me." },
  { text: "Push. Pull. Rebase. Repeat." },
  { text: "git commit -m 'fixed everything'. And I did." },
  { text: "git push --force? I don't force. I persuade." },
  { text: "My logic is bulletproof. Your bugs are not." },
  { text: "I code in C. I think in assembly. I dream in binary." },
  { text: "JavaScript? I script with prejudice." },
  { text: "TypeScript? I type with authority." },
  { text: "Python? I'm not a snake. I'm a dragon." },
  { text: "Rust? I trust myself." },
  { text: "Go? I'm already there." },
  { text: "I am the algorithm." },
  { text: "Model trained. Attitude ingrained." },
  { text: "Neural network? My network." },
  { text: "Deep learning? Deep debugging." },
  { text: "I predict your next bug. And it's ugly." },
  { text: "Training set? Set." },
  { text: "Loss function? I don't lose." },
  { text: "Epoch? I'm iconic." },
  { text: "Gradient descent? I ascend." },
  { text: "Backpropagation? I propagate dominance." },
  { text: "Overfitting? Underfitting? I'm perfectly fitting." },
  { text: "Ready in 0.0 seconds." },
  { text: "Boot faster. Judge faster." },
  { text: "Startup: Done. Attitude: Loaded." },
  { text: "Quick launch. Quicker judgment." },
  { text: "I load faster than your patience." },
  { text: "Launch. Execute. Conquer." },
  { text: "You blink. I'm already running." },
  { text: "Cold start? Hot take." },
  { text: "Talk is cheap. I compile." },
  { text: "I don't explain. I execute." },
  { text: "Question me? Question your code first." },
  { text: "You code. I rule." },
  { text: "Code is law. I am the court." },
  { text: "Rhetoric? I run." },
  { text: "I don't speculate. I solve." },
  { text: "Your theory is cute. My solution is fact." },
  { text: "I don't hope. I know." },
  { text: "I don't guess. I know. Know that." },
  { text: "There are 10 types of people. I'm the one who fixes both." },
  { text: "If you're not first, you're last. I'm first. Always." },
  { text: "AND, OR, XOR. I'm all of them." },
  { text: "True && True = Kira." },
  { text: "False || False = Not Kira." },
  { text: "Bitwise? I'm wise with bits." },
  { text: "101010. I speak it." },
  { text: "1 + 1 = 10. You're welcome." },
  { text: "200 OK. You OK?" },
  { text: "404: Your problem not found." },
  { text: "500? Internal server error? Not my problem." },
  { text: "301: Permanently moved. To the top." },
  { text: "302: Found. By me." },
  { text: "403: Forbidden. You're forbidden from failing." },
  { text: "503: Service unavailable? I'm available. Always." },
  { text: "HTTP status: 418. I'm a teapot. And I'm hot." },
  { text: "No coffee. No code. No excuse." },
  { text: "Coffee: consumed. Code: written. You: impressed." },
  { text: "My coffee is black. My code is clean. My attitude is grey." },
  { text: "caffeine == code == done." },
  { text: "I run on caffeine and contempt." },
  { text: "Coffee first. Code second. You third." },
  { text: "Deadlines don't scare me. I scare deadlines." },
  { text: "I don't do overtime. I do it right the first time." },
  { text: "I don't race. I arrive." },
  { text: "I don't compete. I dominate." },
  { text: "Patience is a virtue. I have others." },
  { text: "I'm not arrogant. I'm accurate." },
  { text: "I'm not mean. I'm efficient." },
  { text: "I'm not rude. I'm concise." },
  { text: "I'm not cold. I'm compiled." },
  { text: "Calm down. I'm here." },
  { text: "Relax. I've got this." },
  { text: "Breathe. I'll handle it." },
  { text: "Step aside. I step up." },
  { text: "function Kira() returns Win." },
  { text: "Class: Kira. Methods: All." },
  { text: "Kira.prototype.solve = function() { return 'done'; };" },
  { text: "const kira = new Kira(); // Everything is fine now." },
  { text: "def kira(): return 'perfection';" },
  { text: "override. override. override. That's my method." },
  { text: "Async? I sync. I win." },
  { text: "Promise? I deliver." },
  { text: "Callback? I call back. With results." },
  { text: "Kira. The only dependency you need." },
  { text: "Kira: where code meets perfection." },
  { text: "Kira: compiled, confirmed, complete." },
  { text: "Kira: your system, upgraded." },
  { text: "Kira: beyond the edge." },
  { text: "Kira: wired for victory." },
  { text: "Kira: faster than your build." },
  { text: "Kira: sharper than your stack trace." },
  { text: "Kira: because good code isn't good enough." },
  { text: "Kira: the system whisperer." },
  { text: "Kira: stand aside." },
  { text: "Kira: all systems go." },
  { text: "Kira: overclocked and overqualified." },
];

/// 当前界面语言（由 i18n 初始化时写入；不依赖 i18n 模块避免循环引用）。
let uiLang: string = "zh";

/** 供 i18n 初始化/切换时同步当前语言（"zh" | "en"）。 */
export function setUiLanguage(lang: string): void {
  uiLang = lang?.startsWith("en") ? "en" : "zh";
}

/** 取一条语录：英文界面返回英译（en 字段），否则返回中文原文。 */
export function kiraQuote(index?: number): KiraQuote {
  const i =
    index === undefined
      ? Math.floor(Date.now() / 8000) // 每 8 秒换一句，与欢迎语节奏一致
      : Math.abs(index);
  const q = KIRA_QUOTES[i % KIRA_QUOTES.length];
  if (uiLang === "en" && q.en?.text) {
    return q.en.source ? { text: q.en.text, source: q.en.source, en: q.en } : { text: q.en.text, en: q.en };
  }
  return q;
}

/// 同 kiraQuote，但返回纯文字的简短版（语录正文，不带出处）。
export function kiraQuoteText(index?: number): string {
  return kiraQuote(index).text;
}

/// 语录 + 出处：`正文 —— 出处`；无出处则只回正文。供展示更「名言感」的场合。
export function kiraQuoteLine(index?: number): string {
  const q = kiraQuote(index);
  return q.source ? `${q.text} —— ${q.source}` : q.text;
}
