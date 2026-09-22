//! 文件树/预览/附件文案词典(features/files、preview、attachments)。

use crate::kits::i18n::entries;

entries! {
    // ── 附件校验(store 域错误码 → 文案)──
    /// 图片数量超限
    too_many_images(n) => ["一条消息最多添加 {n} 张图片", "Up to {n} images per message"],
    /// 图片总大小超限
    images_total_over(limit) => ["图片总大小超过 {limit},请移除部分图片", "Images exceed {limit} in total; remove some first"],
    /// 图片格式不支持
    unsupported_image_type => ["仅支持 PNG、JPG、WebP、GIF 格式的图片", "Only PNG, JPG, WebP, and GIF images are supported"],
    /// 单张图片超限
    image_over(limit) => ["单张图片不能超过 {limit}", "A single image can't exceed {limit}"],
    /// 图片分辨率过大
    pixels_over => ["图片分辨率过大,请压缩后重试", "Image resolution too large; compress it and retry"],
    /// 图片宽高超限
    dims_over(limit) => ["图片宽高不能超过 {limit}px,请缩小后重试", "Image dimensions can't exceed {limit}px; shrink it and retry"],
    /// 图片编码无效
    encode_invalid => ["图片编码无效", "Invalid image encoding"],
    /// 模型不支持图片
    model_no_images => ["当前模型不支持图片,请切换支持图片的模型", "This model doesn't support images — switch to one that does"],
    /// 命令不接受文件附件
    cmd_no_files => ["命令不接受文件附件,请先移除文件", "Commands don't accept file attachments; remove the files first"],
    /// 文件附件无效
    invalid_file => ["文件附件无效,请重新添加后再试", "Invalid file attachment — re-add it and retry"],
    /// 发送失败兜底
    send_failed => ["图片发送失败,请重新添加图片后再试", "Couldn't send the images — re-add them and retry"],
    /// 拖放悬停提示
    drop_add => ["松开以添加附件", "Release to attach"],

    // ── 文件树 ──
    /// 目录已消失
    dir_gone => ["这个目录不在了。可能已被移动或删除。", "This directory is gone. It may have been moved or deleted."],
    /// 目录在工作区外
    dir_outside => ["这个目录在工作区之外，侧栏不会读取它。", "This directory is outside the workspace; the sidebar won't read it."],
    /// 不是目录
    dir_not_dir => ["这不是一个目录。", "This isn't a directory."],
    /// 读取失败
    read_failed(msg) => ["读取失败：{msg}", "Failed to read: {msg}"],
    /// 目录读取中占位
    loading => ["正在读取…", "Loading…"],
    /// 空目录
    empty_dir => ["空目录", "Empty directory"],
    /// 条目截断说明
    truncated => ["条目太多，只显示了一部分。", "Too many entries; showing a subset."],
    /// 会话无工作区
    no_workspace => ["这个会话没有工作区目录。", "This session has no workspace directory."],

    // ── 预览 ──
    /// 文件已消失
    file_gone => ["文件不存在，可能已被移动或删除", "The file is gone; it may have been moved or deleted"],
    /// 文件过期
    file_stale => ["文件已更新，当前显示为旧内容", "The file changed; showing stale content"],
    /// 重新载入
    reload => ["重新载入", "Reload"],
    /// 格式不支持预览
    unsupported_format => ["该格式文件暂时无法预览", "This format can't be previewed"],
    /// 非普通文件
    not_regular => ["该路径不是普通文件，没有可显示的内容", "This path isn't a regular file; nothing to display"],
    /// 单页超限
    page_over(limit) => ["单页内容超过 {limit} 上限，无法读取", "The page exceeds the {limit} cap; can't read"],
    /// 图片显示失败
    image_failed => ["无法显示这张图片", "Couldn't display this image"],
    /// PDF 需要密码
    pdf_password => ["此 PDF 需要密码，暂不支持预览", "This PDF is password-protected; preview isn't supported"],
    /// PDF 显示失败
    pdf_failed(msg) => ["无法显示 PDF：{msg}", "Couldn't display the PDF: {msg}"],
    /// PDF 显示失败:位图尺寸不符 detail
    pdf_bitmap_mismatch => ["位图尺寸不符", "bitmap size mismatch"],
    /// PDF 整面失败兜底(无 detail)
    pdf_display_failed => ["无法显示这份 PDF", "Couldn't display this PDF"],
    /// PDF 解析失败
    pdf_invalid => ["无法解析这份 PDF", "Couldn't parse this PDF"],
    /// PDF 过大
    pdf_too_large(objects, pages) => ["PDF 过大(对象 {objects} / 页 {pages})", "PDF too large ({objects} objects / {pages} pages)"],
    /// 无此页
    no_such_page(page) => ["没有第 {page} 页", "No page {page}"],
    /// PDF 页绘制中
    pdf_drawing(page) => ["PDF 第 {page} 页：正在绘制页面…", "PDF page {page}: rendering…"],
    /// 加载更多
    load_more => ["加载更多", "Load more"],
}
