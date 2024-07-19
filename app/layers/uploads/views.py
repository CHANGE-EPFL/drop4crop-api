from fastapi import (
    Depends,
    APIRouter,
    Request,
    Query,
    BackgroundTasks,
    Header,
    Response,
    HTTPException,
)
from uuid import uuid4
from typing import Any, Annotated
from collections import defaultdict
from app.auth import require_admin, User

router = APIRouter()

# In-memory storage for file parts
file_storage = defaultdict(dict)


@router.post("")
async def upload_file(
    request: Request,
    upload_length: int = Header(..., alias="Upload-Length"),
    content_type: str = Header(..., alias="Content-Type"),
    transect_id: str = Header(None, alias="Transect-Id"),
    *,
    background_tasks: BackgroundTasks,
    user: User = Depends(require_admin),
) -> str:
    try:
        # Generate a unique object id
        object_id = str(uuid4())

        # Initialize in-memory storage
        file_storage[object_id] = {
            "upload_length": upload_length,
            "parts": [],
            "data": bytearray(upload_length),
            "user": user,
            "transect_id": transect_id,
            "content_type": content_type,
        }

    except Exception as e:
        print(e)
        raise HTTPException(
            status_code=500,
            detail="Failed to create object",
        )

    return object_id


@router.patch("")
async def upload_chunk(
    request: Request,
    patch: str = Query(...),
    upload_offset: int = Header(..., alias="Upload-Offset"),
    upload_length: int = Header(..., alias="Upload-Length"),
    upload_name: str = Header(..., alias="Upload-Name"),
    content_type: str = Header(..., alias="Content-Type"),
    content_length: int = Header(..., alias="Content-Length"),
    *,
    background_tasks: BackgroundTasks,
    user: User = Depends(require_admin),
):
    """Handle chunked file upload"""

    # Clean " from patch
    patch = patch.replace('"', "")

    # Retrieve the object from in-memory storage
    if patch not in file_storage:
        raise HTTPException(
            status_code=404,
            detail="Object not found",
        )

    object = file_storage[patch]

    final_part = False
    # Get the part number from the offset
    if upload_length - upload_offset == content_length:
        last_part = (
            object["parts"][-1] if object["parts"] else {"PartNumber": 0}
        )
        part_number = last_part["PartNumber"] + 1
    else:
        part_number = (int(upload_offset) // int(content_length)) + 1

    if upload_offset + content_length == upload_length:
        final_part = True

    # Get the chunk data
    data = await request.body()

    print(
        f"Working on part number {part_number}, chunk {upload_offset} "
        f"{int(upload_offset)+int(content_length)} "
        f"of {upload_length} bytes "
        f"({int(upload_offset)/int(upload_length)*100}%)"
    )

    try:
        object["parts"].append(
            {
                "PartNumber": part_number,
                "Size": content_length,
                "Offset": upload_offset,
                "Length": content_length,
            }
        )
        object["data"][upload_offset : upload_offset + content_length] = data

        if final_part:
            # Complete the upload
            file_info = {
                "id": patch,
                "size": len(object["data"]),
                "upload_name": upload_name,
                "parts_count": len(object["parts"]),
            }
            print(file_info)
            return file_info

    except Exception as e:
        print(e)
        raise HTTPException(
            status_code=500,
            detail=f"Failed to upload file: {e}",
        )

    return {"message": "Chunk uploaded successfully"}


@router.head("")
async def check_uploaded_chunks(
    response: Response,
    patch: str = Query(...),
    user: User = Depends(require_admin),
):
    """Responds with the offset of the next expected chunk"""

    # Clean " from patch
    patch = patch.replace('"', "")

    if patch not in file_storage:
        raise HTTPException(
            status_code=404,
            detail="Object not found",
        )

    object = file_storage[patch]

    next_expected_offset = 0
    if object["parts"]:
        last_part = object["parts"][-1]
        next_expected_offset = last_part["Offset"] + last_part["Length"]

    # Return headers with Upload-Offset
    response.headers["Upload-Offset"] = str(next_expected_offset)

    return
