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
from app.db import get_session, AsyncSession
from sqlmodel import select
from app.layers.models import Layer
from app.geoserver.services import process_and_upload_geotiff

router = APIRouter()

# In-memory storage for file parts
file_storage = defaultdict(dict)


@router.post("")
async def upload_file(
    request: Request,
    upload_length: int = Header(..., alias="Upload-Length"),
    content_type: str = Header(..., alias="Content-Type"),
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
            "content_type": content_type,
        }

        print(f"Created object {object_id} with {upload_length} bytes")

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
    session: AsyncSession = Depends(get_session),
):
    """Handle chunked file upload"""

    # Clean " from patch
    patch = patch.replace('"', "")
    # Extract filename into variables. Structure is:
    # {crop}_{watermodel}_{climatemodel}_{scenario}_{variable}_{year}.tif
    try:
        crop, water_model, climate_model, scenario, variable, year = (
            # Remove file extension then split by _
            upload_name.lower()
            .split(".")[0]
            .split("_")
        )
        print(crop, water_model, climate_model, scenario, variable, year)
    except Exception as e:
        print(e)
        raise HTTPException(
            status_code=400,
            detail={
                "message": "Invalid filename, must be in the format "
                "{crop}_{watermodel}_{climatemodel}_{scenario}_"
                "{variable}_{year}.tif",
            },
        )

    # Query DB to check if the layers exist, if they do do not accept the
    query = select(Layer).where(
        Layer.crop == crop,
        Layer.water_model == water_model,
        Layer.climate_model == climate_model,
        Layer.scenario == scenario,
        Layer.variable == variable,
        Layer.year == int(year),
    )

    layer = await session.exec(query)
    layer = layer.one_or_none()

    if layer:
        raise HTTPException(
            status_code=400,
            detail={
                "message": "Layer already exists",
            },
        )

    # Retrieve the object from in-memory storage
    if patch not in file_storage:
        print(
            f"Layer exists ! Crop: {crop}, Water Model: {water_model}, "
            f"Climate Model: {climate_model}, Scenario: {scenario}, "
            f"Variable: {variable}, Year: {year}"
        )
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
            print("LET'S UPLOAD TO GEOSERVER HERE")
            layer_name = (
                f"{crop}_{water_model}_{climate_model}_"
                f"{scenario}_{variable}_{year}"
            )
            res = await process_and_upload_geotiff(
                object["data"],
                layer_name,
            )
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
