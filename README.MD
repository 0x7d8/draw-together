# draw-together

something you should not install on your servers unless you hate them.
*(really hate them)*

## Install

```sh
git clone https://github.com/0x7d8/draw-together.git
cd draw-together

# make sure nodejs 18+ is installed
npm i -g pnpm
pnpm install
pnpm build
```

## Run

### Save to Disk

```sh
cd draw-together

pnpm prod
```

### Do not Save to Disk

```sh
cd draw-together

pnpm prod --nosave
```
