import type { Messages } from './en';

export const zh: Messages = {
  htmlLang: 'zh-CN',
  locale: 'zh',
  a11y: {
    skip: '跳到正文',
    langSwitch: '语言',
  },
  meta: {
    title: 'robroz：用聊天驱动的单机私有化多智能体工作台',
    description:
      '在聊天里把任务交给 bro。每个 bro 在你本机的 Docker 沙箱里干活，用卡片汇报进度和结果。私有化部署，Rust 编写。',
  },
  nav: {
    lang: { en: 'EN', zh: '中文' },
    github: 'GitHub',
    wiki: 'Wiki',
  },
  hero: {
    title: '在聊天里把任务交给 bro，活在你自己的电脑上干。',
    lede: 'robroz 是单机私有化的多智能体协同工作台，交互是聊天，不是 IDE。每个 bro（agent）在你本机的 Docker 沙箱里干活，进度、审批和结果都以卡片回到对话里。',
    cta: {
      primary: '快速开始',
      secondary: '在 GitHub 上查看',
    },
    demo: {
      you: '你',
      bro: 'Ada',
      user: '查一下 Rust 最近一个版本的新功能，写成三点',
      userGloss: '',
      status: {
        queued: '排队',
        queuedGloss: '',
        running: '进行',
        running1Progress: '在想怎么回',
        running1Gloss: '',
        running2Progress: '在用 web_search',
        running2Gloss: '',
        done: '完成',
        doneGloss: '',
      },
      delivery: '已写入结果',
      deliveryPath: 'tasks/<taskId>/result.txt',
      deliveryGloss: '',
      caption: '一个任务一张状态卡，原地更新。结果是群目录里的一个文件。',
    },
  },
  features: {
    title: 'bro 现在能做什么',
    tasks: {
      title: '任务卡',
      body: '在群里以 `@名字` 开头，就是给这个 bro 派任务；私聊里每条消息都是任务。每个任务一张状态卡，从排队到进行再到完成，跑的时候带一行简短进度。结果以交付卡送回。例行任务到点也这样交付简报。',
    },
    memory: {
      title: '记忆',
      body: 'bro 可以提议记下一条关于你的事：你是谁、你的工作习惯，或项目记录。你在确认卡上点「记下」才算数。你也可以在「记忆」页自己记、改、忘掉。目前是基础版：记忆只拼进 bro 的提示，不开放任何权限。',
    },
    skills: {
      title: '技能',
      body: '技能是一段有名字的做法说明，比如你习惯的周报写法。bro 可以提议一项技能，你在确认卡上点「保存」，再给要用它的群打开开关。目前是基础版：技能只是提示里的文字，不开放任何权限。',
    },
  },
  control: {
    title: '你说了算',
    lede: 'bro 读写自己群目录里的东西直接放行。凡是要往外伸手的，都会停下来等你。',
    tiers: {
      title: '三级权限',
      body: '读写群目录直接放行；出网只许 GET，并按任务计次（默认 8 次）；对外写入和验证码会停下任务，等人处理。',
    },
    approval: {
      title: '审批卡',
      body: '任务跑到对外写入时会暂停，宿主机发一张审批卡，状态卡显示「等你批准」。点「批准」，任务重新排队并接着跑完；点「拒绝」，任务结束，那一步不做。之后卡上写明谁、什么时候回答的，比如「我 批准 · 14:02」。',
    },
    takeover: {
      title: '接管',
      body: '有些步骤得由人来，比如验证码。卡住时会停下来等人接管。',
    },
    sandbox: {
      title: '沙箱',
      body: '每个任务都在没有网络的 Docker 容器里跑。搜索和读网页由宿主机替 bro 去做，同样算进每个任务的出网次数。你的搜索密钥只留在本机。',
    },
    demo: {
      status: '等你批准',
      statusGloss: '',
      approval: '要对外写入：…',
      approvalGloss: '',
      approve: '批准',
      approveGloss: '',
      deny: '拒绝',
      denyGloss: '',
      answered: '我 批准 · 14:02',
      answeredGloss: '',
      stuck: '卡住',
      stuckGloss: '',
    },
  },
  quickstart: {
    title: '自己部署',
    lede: '需要 Rust stable 工具链和 Docker。在仓库根目录：',
    code: `# 宿主机和桌面客户端
cargo build -p robroz-host -p robroz-client
# 跑任务用的 runner 镜像
docker build -f runner/Dockerfile -t robroz-runner:local .
# 只监听 127.0.0.1:8790，不用令牌
cargo run -p robroz-host
# 另开一个终端
cargo run -p robroz-client`,
    next: '在客户端「聊天」标题旁点「引入一个 bro」，进它的私聊发一条消息，这就是第一个任务。没接模型时，bro 会把你的话原样写成结果。',
    more: '接 OpenAI 兼容模型、设置联网搜索、设令牌、从别的机器访问宿主机，见安装文档。',
    moreLink: {
      label: '安装文档',
      href: 'https://github.com/PandaQuQ/robroz/blob/main/docs/install.md',
    },
    platforms:
      '宿主机、执行器（runner）和桌面客户端全部用 Rust 写。客户端是 Windows、Linux、macOS 上的原生程序，不套网页。',
  },
  links: {
    title: '了解更多',
    github: {
      label: 'GitHub',
      body: '源代码、Issue 和 PR。',
      url: 'https://github.com/PandaQuQ/robroz',
    },
    wiki: {
      label: 'Wiki',
      body: '给人看的：安装、日常使用、配置、常见问题和路线图。',
      url: 'https://github.com/PandaQuQ/robroz/wiki',
    },
    agents: {
      label: 'AGENTS.md',
      body: '给编码 agent 和贡献者看的：仓库规则、接口契约和代码位置。',
      url: 'https://github.com/PandaQuQ/robroz/blob/main/AGENTS.md',
    },
  },
  footer: {
    license: '许可证：待定。',
    repo: 'robroz 的 GitHub 仓库',
  },
};
