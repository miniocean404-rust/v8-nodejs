async function main() {
  const file = await fs.openFile("./text.txt")
  const fileContent = await file.content()
  await file.seek(0)
  await file.write(fileContent + "\n hello world")
  print(await file.content())
}
