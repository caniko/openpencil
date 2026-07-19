# HTML 网页快照导入

静态 HTML 抓取不执行 JavaScript，因此 SPA 或复杂 CSS 页面应使用网页快照链路。抽取脚本在你自己的浏览器里读取 `getComputedStyle` 和页面布局，再生成 OpenPencil 快照 v1 JSON；它不会发起网络请求，也不会把元素插入被抽取的页面。

## 三步使用

1. 在 Chrome 等浏览器中打开目标页面，等待页面进入希望保存的状态。打开 DevTools Console。
2. 将 [`snapshot-extractor.js`](../crates/op-html/assets/snapshot-extractor.js) 的完整内容粘贴进 Console 并回车。脚本会尝试把 JSON 写入剪贴板，同时下载 `snapshot.json`；其中任一种成功即可。
3. 导入快照：

   ```bash
   # 导入正在运行的编辑器
   op import:snapshot snapshot.json

   # 不启动编辑器，直接生成 .op 文件
   op import:snapshot snapshot.json --out snapshot.op
   ```

   MCP 客户端也可以调用 `import_web_snapshot`，传 `snapshot` JSON 文本或 `snapshotPath` 文件路径。

部分浏览器会在首次向 Console 粘贴时显示自助式粘贴保护提示；请只按浏览器给出的本地提示操作，并确认粘贴的是仓库中的原始脚本。

## 书签脚本

可以把脚本压缩为单行，并在前面加 `javascript:` 后保存为书签。浏览器对书签 URL 有长度限制，因此本项目不承诺托管或自动更新书签版本；升级 OpenPencil 后应从当前仓库重新生成。

## 已知限制

- 跨域 iframe 的内部内容不会被抽取。
- CORS 污染的图片无法经 canvas 内嵌，会保留远程 URL 并标记为 `tainted`；导入结果会产生汇总警告。
- 动画、视频和 canvas 只捕获执行脚本时的当前帧。
- 图片最长边会缩到 2048 CSS 像素；全页 data URL 达到 24 MiB 后，后续图片保留 URL 或使用占位图。
- 最多抽取 20,000 个可见节点。达到上限时 JSON 带 `truncated: true`。
- 快照转换保持绝对定位，不尝试从像素盒反推 flex/auto-layout。

人工冒烟可在 `https://example.com` 执行脚本，将产物保存为 `/tmp/snapshot.json`，再运行：

```bash
cargo run -p op-cli -- import:snapshot /tmp/snapshot.json --out /tmp/snapshot.op
```
