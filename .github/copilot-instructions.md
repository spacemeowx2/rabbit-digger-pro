## 全局规则

- 使用中文进行思考和交流，使用英文进行编码和注释
- 在完成任务前验证产生的编辑是否有错误

## 修改 /ui/ 目录下的文件

- 不要直接用 fetch 获取资源，而是使用 ui/src/api/v1.ts 来获取数据。
- 使用 2 spaces 缩进。使用单引号。
- 对于动态的 className，优先使用 clsx 来生成。只有遇到可能覆盖的情况下才使用 `import { cn } from '@/lib/utils'`。cn 底层用的是 twMerge，性能上会比 clsx 差一些。
