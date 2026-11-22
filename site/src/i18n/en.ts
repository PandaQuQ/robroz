export const en = {
  htmlLang: 'en',
  locale: 'en',
  a11y: {
    skip: 'Skip to content',
    langSwitch: 'Language',
  },
  meta: {
    title: 'robroz: a self-hosted multi-agent workspace you drive through chat',
    description:
      'Hand tasks to bros in chat. Each one works in a Docker sandbox on your machine and reports back with cards. Self-hosted, written in Rust.',
  },
  nav: {
    lang: { en: 'EN', zh: '中文' },
    github: 'GitHub',
    wiki: 'Wiki',
  },
  hero: {
    title: 'Hand tasks to bros in chat. They work on your machine.',
    lede: 'robroz is a self-hosted, single-machine multi-agent workspace that you drive through chat, not an IDE. Each bro (agent) works in a Docker sandbox on your computer, and progress, approvals and results come back to the conversation as cards.',
    cta: {
      primary: 'Quick start',
      secondary: 'View on GitHub',
    },
    demo: {
      you: 'You',
      bro: 'Ada',
      user: '查一下 Rust 最近一个版本的新功能，写成三点',
      userGloss: "Look up what's new in the latest Rust release, in three points",
      status: {
        queued: '排队',
        queuedGloss: 'Queued',
        running: '进行',
        running1Progress: '在想怎么回',
        running1Gloss: 'Running · Thinking',
        running2Progress: '在用 web_search',
        running2Gloss: 'Running · Using web_search',
        done: '完成',
        doneGloss: 'Done',
      },
      delivery: '已写入结果',
      deliveryPath: 'tasks/<taskId>/result.txt',
      deliveryGloss: 'Result written',
      caption: 'One status card per task, updated in place. The result is a file in the group\'s folder.',
    },
  },
  features: {
    title: 'What a bro can do today',
    tasks: {
      title: 'Task cards',
      body: 'In a group, start a message with `@name` to give that bro a task; in a DM every message is a task. Each task gets one status card that moves from queued to running to done, with a short progress line while it runs. The result arrives as a delivery card. Scheduled routines deliver a briefing the same way.',
    },
    memory: {
      title: 'Memory',
      body: 'A bro can propose something to remember about you: who you are, how you work, or a project note. It only counts after you press 「记下」 (*Keep*) on the confirm card. You can also add, edit and forget memories yourself on the Memory page. A basic version: memories are added to the bro\'s prompt, and grant no permissions.',
    },
    skills: {
      title: 'Skills',
      body: 'A skill is a named set of instructions, such as how you like a weekly report written. A bro can propose one; you save it from the confirm card, then switch it on for the groups that should use it. A basic version: skills are prompt text only, and grant no permissions.',
    },
  },
  control: {
    title: "You're in control",
    lede: "Bros read and write their group's folder freely. Anything that reaches outside stops and waits for you.",
    tiers: {
      title: 'Three permission tiers',
      body: 'Reading and writing the group folder is allowed. Outbound network is GET-only and counted per task (8 by default). External writes and captchas stop the task and wait for a person.',
    },
    approval: {
      title: 'Approval cards',
      body: 'When a task reaches an external write, it pauses and the host posts an approval card. The status card reads 「等你批准」 (*Waiting for your approval*). Press 「批准」 (*Approve*) and the task queues again and finishes; press 「拒绝」 (*Deny*) and it ends without doing that step. The card then shows who answered and when, for example 「我 批准 · 14:02」 (*I approved · 14:02*).',
    },
    takeover: {
      title: 'Takeover',
      body: 'Some steps need a person, such as a captcha. When a task gets stuck, it stops and waits for someone to take over.',
    },
    sandbox: {
      title: 'Sandbox',
      body: "Every task runs in a Docker container with no network. Web search and page reads are made by the host on the bro's behalf and count towards the same per-task limit. Your search key stays on your machine.",
    },
    demo: {
      status: '等你批准',
      statusGloss: 'Waiting for your approval',
      approval: '要对外写入：…',
      approvalGloss: 'External write: …',
      approve: '批准',
      approveGloss: 'Approve',
      deny: '拒绝',
      denyGloss: 'Deny',
      answered: '我 批准 · 14:02',
      answeredGloss: 'I approved · 14:02',
      stuck: '卡住',
      stuckGloss: 'Stuck',
    },
  },
  quickstart: {
    title: 'Self-host it',
    lede: 'You need the Rust stable toolchain and Docker. From the repository root:',
    code: `# host and desktop client
cargo build -p robroz-host -p robroz-client
# runner image used for tasks
docker build -f runner/Dockerfile -t robroz-runner:local .
# listens on 127.0.0.1:8790 only, no token needed
cargo run -p robroz-host
# in a second terminal
cargo run -p robroz-client`,
    next: 'In the client, click 「引入一个 bro」 (*add a bro*) next to the 「聊天」 (*Chats*) heading, open its DM and send a message: that is your first task. robroz has no default model: you choose one when you deploy the host. Until you do, the result reads 「还没设置模型」 (*the model isn\'t set up yet*).',
    more: 'To choose an OpenAI-compatible model, set up web search, set a token or reach the host from another machine, see the install guide.',
    moreLink: {
      label: 'Install guide',
      href: 'https://github.com/PandaQuQ/robroz/blob/main/docs/install.md',
    },
    platforms:
      'The host, the runner and the desktop client are written in Rust. The client is native on Windows, Linux and macOS, with no web view.',
  },
  links: {
    title: 'Read more',
    github: {
      label: 'GitHub',
      body: 'Source code, issues and pull requests.',
      url: 'https://github.com/PandaQuQ/robroz',
    },
    wiki: {
      label: 'Wiki',
      body: 'For people: installing, everyday use, configuration, FAQ and roadmap.',
      url: 'https://github.com/PandaQuQ/robroz/wiki',
    },
    agents: {
      label: 'AGENTS.md',
      body: 'For coding agents and contributors: repository rules, the API contract and where the code lives.',
      url: 'https://github.com/PandaQuQ/robroz/blob/main/AGENTS.md',
    },
  },
  footer: {
    license: 'License: AGPL-3.0',
    repo: 'robroz on GitHub',
  },
};

export type Messages = typeof en;
