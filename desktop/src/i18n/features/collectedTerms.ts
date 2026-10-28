/** Complete dictation.collectedTerms subtree; root merges it into each locale. */
export const collectedTermsMessages: {
  en: Record<string, unknown>;
  zh: Record<string, unknown>;
} = {
  en: {
    title: "Collected words",
    description:
      "Words found in at least two retained transcripts appear automatically. Like words to keep them, or delete them from this list.",
    boundary:
      "Collection and likes stay local and do not change dictation or AI prompts. Counts refer to retained transcripts, not lifetime usage or accuracy.",
    coverage:
      "Uses local Chinese dictionary segmentation and English technical terms. Coverage is limited; unknown names may be missed. Paths, URLs and code are skipped. Text may already include your confirmed vocabulary corrections.",
    search: "Search collected words",
    refresh: "Refresh",
    retry: "Retry",
    filterLabel: "Word views",
    filters: { all: "All", liked: "Liked", deleted: "Deleted" },
    empty: {
      all: "No matching words yet. Words appear after two retained transcripts; liked words stay visible below that threshold.",
      liked: "No matching liked words.",
      deleted:
        "No matching deleted words. Deleted words stay hidden until you restore them.",
    },
    like: "Like",
    unlike: "Unlike",
    delete: "Delete",
    restore: "Restore",
    likeTerm: "Like {{term}}",
    unlikeTerm: "Unlike {{term}}",
    deleteTerm: "Delete {{term}}",
    restoreTerm: "Restore {{term}}",
    recordCount_one: "{{count}} retained transcript",
    recordCount_other: "{{count}} retained transcripts",
    page: "Page {{page}} of {{pages}} · {{count}} words",
    previous: "Previous",
    next: "Next",
    loadFailed: "Could not load collected words.",
    changeFailed: "Could not save this change. Refresh and try again.",
    confirmedTitle: "Confirmed vocabulary and corrections",
    confirmedHint:
      "These existing rules still apply to dictation. They are separate from collected words and likes.",
  },
  zh: {
    title: "自动词表",
    description:
      "在至少两条保留转录中出现的词会自动收集。可点赞保留，也可从列表中删除。",
    boundary:
      "收录和点赞仅保存在本地，不改变输入正文或 AI 提示。次数指当前保留转录的记录数，不是终身使用次数或正确率。",
    coverage:
      "采用本地中文词典分词和英文技术词提取，覆盖有限，生僻人名可能遗漏；跳过路径、网址与代码。来源文字可能已包含旧词库的确认纠正。",
    search: "搜索词表",
    refresh: "刷新",
    retry: "重试",
    filterLabel: "词表视图",
    filters: { all: "全部", liked: "已赞", deleted: "已删除" },
    empty: {
      all: "暂无匹配词项。在两条保留转录中出现后会自动展示；点赞词不受此门槛限制。",
      liked: "暂无匹配的已赞词。",
      deleted: "暂无匹配的已删除词。删除后不会自动重新展示，可在此恢复。",
    },
    like: "点赞",
    unlike: "取消赞",
    delete: "删除",
    restore: "恢复",
    likeTerm: "点赞 {{term}}",
    unlikeTerm: "取消点赞 {{term}}",
    deleteTerm: "删除 {{term}}",
    restoreTerm: "恢复 {{term}}",
    recordCount: "出现在 {{count}} 条保留转录中",
    page: "第 {{page}} / {{pages}} 页 · 共 {{count}} 词",
    previous: "上一页",
    next: "下一页",
    loadFailed: "词表加载失败。",
    changeFailed: "未能保存操作，请刷新后重试。",
    confirmedTitle: "已确认词库与主动纠正",
    confirmedHint: "这些已有规则仍用于输入纠正，与自动词表、点赞互相独立。",
  },
};
