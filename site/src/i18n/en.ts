export const en = {
  htmlLang: 'en',
  locale: 'en',
  title: 'robroz',
  description:
    'A self-hosted, single-machine multi-agent workspace that you drive through chat, not an IDE.',
  skipToContent: 'Skip to content',
  lang: {
    en: 'EN',
    zh: '中文',
    switchLabel: 'Language',
  },
  intro: {
    headline:
      'A self-hosted, single-machine multi-agent workspace that you drive through chat, not an IDE.',
  },
  chat: {
    ariaLabel: 'Sample conversation',
    you: 'You',
    bro: 'bro',
    userMessage:
      '@docs Rewrite the landing-page copy and propose a memory for the tone of voice.',
    broMessage:
      'Draft is in the delivery card. I also proposed a memory for the tone of voice — confirm it when you are ready.',
    statusTitle: 'Rewrite landing-page copy',
    status: {
      queued: 'Queued',
      running: 'Running',
      done: 'Done',
    },
  },
  capabilities: {
    title: 'Three capabilities',
    taskCards: {
      title: 'Task cards',
      body: 'Status and delivery cards come back in the conversation as a task moves from queued to running to done.',
    },
    memory: {
      title: 'Memory',
      body: 'A bro can propose something to remember. It takes effect only after you confirm it.',
    },
    skills: {
      title: 'Skills',
      body: 'Reusable know-how is added the same way: the bro proposes a skill, and you decide whether to keep it.',
    },
  },
  control: {
    title: "You're in control",
    lead: 'External writes, captchas and other sensitive steps stop and wait for a person. Nothing important goes through on its own.',
    approval: {
      title: 'Approval cards',
      body: 'When a bro needs a decision, an approval or choice card lands in the chat. The task waits until you answer.',
    },
    takeover: {
      title: 'Takeover',
      body: 'If a task is blocked, you can take it over in a direct message, finish the step yourself, and let the bro continue.',
    },
  },
  quickStart: {
    title: 'Self-host, quickly',
    lead: 'You need the Rust stable toolchain and Docker. From the repository root:',
    after:
      'In the client, add a bro, open its DM and send a message: that is your first task.',
    code: `cargo build -p robroz-host -p robroz-client
docker build -f runner/Dockerfile -t robroz-runner:local .
cargo run -p robroz-host
cargo run -p robroz-client`,
  },
  links: {
    title: 'Source and docs',
    github: { label: 'GitHub', href: 'https://github.com/PandaQuQ/robroz' },
    wiki: { label: 'Wiki', href: 'https://github.com/PandaQuQ/robroz/wiki' },
    agents: {
      label: 'AGENTS.md',
      href: 'https://github.com/PandaQuQ/robroz/blob/main/AGENTS.md',
    },
  },
  footer: 'Placeholder copy. Final wording will match the README.',
};

export type Messages = typeof en;
