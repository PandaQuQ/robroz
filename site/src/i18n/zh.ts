import type { Messages } from './en';

export const zh: Messages = {
  htmlLang: 'zh-CN',
  locale: 'zh',
  title: 'robroz',
  description: '单机私有化的多智能体协同，用聊天来派任务，而不是用 IDE。',
  skipToContent: '跳到正文',
  lang: {
    en: 'EN',
    zh: '中文',
    switchLabel: '语言',
  },
  intro: {
    headline: '单机私有化的多智能体协同，用聊天来派任务，而不是用 IDE。',
  },
  chat: {
    ariaLabel: '示例对话',
    you: '你',
    bro: 'bro',
    userMessage: '@docs 重写落地页文案，并提议一条关于语气的记忆。',
    broMessage: '草稿在交付卡里。我也提议了一条关于语气的记忆，确认后才会生效。',
    statusTitle: '重写落地页文案',
    status: {
      queued: '排队',
      running: '进行中',
      done: '完成',
    },
  },
  capabilities: {
    title: '三个能力',
    taskCards: {
      title: '任务卡',
      body: '任务从排队到进行中再到完成，状态卡和交付卡都会回到对话里。',
    },
    memory: {
      title: '记忆',
      body: 'bro 可以提议一条记忆，只有你确认之后才会生效。',
    },
    skills: {
      title: '技能',
      body: '可复用的做法也一样：bro 提议一项技能，由你决定要不要留下。',
    },
  },
  control: {
    title: '你说了算',
    lead: '对外写入、验证码和其他敏感步骤会停下来等人处理，重要的事不会自己通过。',
    approval: {
      title: '审批卡',
      body: 'bro 需要你做决定时，聊天里会出现审批卡或选择卡，任务会等到你回答。',
    },
    takeover: {
      title: '接管',
      body: '任务卡住时，你可以在私聊里接管，自己做完那一步，再让 bro 继续。',
    },
  },
  quickStart: {
    title: '自托管快速开始',
    lead: '需要 Rust 稳定版工具链和 Docker。在仓库根目录执行：',
    after: '打开客户端，引入一个 bro，进入它的私聊并发送一条消息，这就是第一个任务。',
    code: `cargo build -p robroz-host -p robroz-client
docker build -f runner/Dockerfile -t robroz-runner:local .
cargo run -p robroz-host
cargo run -p robroz-client`,
  },
  links: {
    title: '源码和文档',
    github: { label: 'GitHub', href: 'https://github.com/PandaQuQ/robroz' },
    wiki: { label: 'Wiki', href: 'https://github.com/PandaQuQ/robroz/wiki' },
    agents: {
      label: 'AGENTS.md',
      href: 'https://github.com/PandaQuQ/robroz/blob/main/AGENTS.md',
    },
  },
  footer: '文案为占位，正式措辞会与 README 对齐。',
};
