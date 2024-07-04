from app.layers.models import (
    Layer,
    LayerCreate,
    LayerRead,
)
from app.db import get_session, AsyncSession
from fastapi import (
    Depends,
    APIRouter,
    Query,
    BackgroundTasks,
)
from uuid import UUID
from app.crud import CRUD
from app.layers.services import (
    get_count,
    get_data,
    get_one,
    create_one,
    update_one,
    crud,
    get_layers,
)
from typing import Any

router = APIRouter()


@router.get("/{layer_id}", response_model=LayerRead)
async def get_layer(
    obj: CRUD = Depends(get_one),
) -> LayerRead:
    """Get a layer by id"""

    return obj


@router.get("")
async def get_all_layers(
    crop: str = Query(None),
    water_model: str = Query(None),
    climate_model: str = Query(None),
    scenario: str = Query(None),
    variable: str = Query(None),
    year: int = Query(None),
    layers: dict[tuple[str, str, str, str, str, int], LayerRead] = Depends(
        get_layers
    ),
) -> Any:
    """Get all Layer data"""
    results = [
        layer
        for key, layer in layers.items()
        if (crop is None or crop.lower() == key[0])
        and (water_model is None or water_model.lower() == key[1])
        and (climate_model is None or climate_model.lower() == key[2])
        and (scenario is None or scenario.lower() == key[3])
        and (variable is None or variable.lower() == key[4])
        and (year is None or year == key[5])
    ]

    print("Result count:", len(results))
    return results


@router.post("", response_model=LayerRead)
async def create_layer(
    create_obj: LayerCreate,
    background_tasks: BackgroundTasks,
    session: AsyncSession = Depends(get_session),
) -> LayerRead:
    """Creates a layer data record"""

    obj = await create_one(create_obj.model_dump(), session, background_tasks)

    return obj


@router.put("/{layer_id}", response_model=LayerRead)
async def update_layer(
    updated_layer: Layer = Depends(update_one),
) -> LayerRead:
    """Update a layer by id"""

    return updated_layer


@router.delete("/batch", response_model=list[str])
async def delete_batch(
    ids: list[UUID],
    session: AsyncSession = Depends(get_session),
) -> list[str]:
    """Delete by a list of ids"""

    for id in ids:
        obj = await crud.get_model_by_id(model_id=id, session=session)
        if obj:
            await session.delete(obj)

    await session.commit()

    return [str(obj_id) for obj_id in ids]


@router.delete("/{layer_id}")
async def delete_layer(
    layer: LayerRead = Depends(get_one),
    session: AsyncSession = Depends(get_session),
) -> None:
    """Delete a layer by id"""

    await session.delete(layer)
    await session.commit()

    return {"ok": True}
