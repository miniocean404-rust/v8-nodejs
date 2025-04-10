import fs from "fs"
import { createFileContent } from "./utils"

export async function main() {
  const dirname = import.meta.dirname
  const file = await fs.openFile(dirname + "/text.txt")
  const fileContent = await file.content()
  await file.seek(0)
  await file.write(createFileContent(fileContent))
  print(await file.content())
}
